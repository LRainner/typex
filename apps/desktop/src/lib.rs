use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use futures::{StreamExt, stream};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use tauri::{
    Emitter, Manager, WindowEvent,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use typex_config::{AppConfig, AsrConnection, LlmConnection};
use typex_core::typex_asr::AsrProvider;
use typex_core::typex_llm::LlmProvider;
use typex_core::{TypeX, TypeXBuildOptions, build_typex_from_config};

/// The spawn_blocking task returns this handle once the recording stop signal is received.
/// Awaiting it yields the accumulator JoinHandle, which in turn yields the PCM data.
type RecordingAccFuture =
    tokio::task::JoinHandle<anyhow::Result<tokio::task::JoinHandle<anyhow::Result<Vec<u8>>>>>;

struct AppState {
    config: Mutex<AppConfig>,
    config_path: std::path::PathBuf,
    provider_status_path: std::path::PathBuf,
    db: Mutex<Connection>,
    pipeline: Mutex<Arc<TypeX>>,
    capture: Mutex<Option<typex_audio::MicrophoneCapture>>,
    /// Send a value to signal the spawn_blocking recording task to stop.
    recording_stop: Mutex<Option<std::sync::mpsc::Sender<()>>>,
    /// The spawn_blocking task that holds the accumulator JoinHandle.
    recording_acc_future: Mutex<Option<RecordingAccFuture>>,
    /// Currently registered shortcut string.
    shortcut: Mutex<String>,
    overlay_error_token: AtomicU64,
    overlay_save_token: AtomicU64,
    /// Prevents concurrent recording start attempts.
    recording_starting: AtomicBool,
    /// Tokio runtime handle — needed because global shortcut callbacks run outside Tokio context.
    rt: tokio::runtime::Handle,
}

fn config_path(app: &tauri::AppHandle) -> std::path::PathBuf {
    app.path()
        .app_config_dir()
        .expect("failed to resolve app config dir")
        .join("config.toml")
}

fn provider_status_path(app: &tauri::AppHandle) -> std::path::PathBuf {
    app.path()
        .app_config_dir()
        .expect("failed to resolve app config dir")
        .join("provider_status.json")
}

fn db_path(app: &tauri::AppHandle) -> std::path::PathBuf {
    app.path()
        .app_config_dir()
        .expect("failed to resolve app config dir")
        .join("typex.db")
}

fn init_db(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS history (
            id INTEGER PRIMARY KEY,
            text TEXT NOT NULL,
            created_at TEXT NOT NULL,
            pinned INTEGER NOT NULL DEFAULT 0
        );",
    )?;
    // Migration: add pinned column to existing tables that lack it
    let has_pinned: bool = conn
        .query_row(
            "SELECT 1 FROM pragma_table_info('history') WHERE name = 'pinned'",
            [],
            |_| Ok(true),
        )
        .unwrap_or(false);

    if !has_pinned {
        conn.execute_batch("ALTER TABLE history ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0")?;
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HistoryEntry {
    id: i64,
    text: String,
    created_at: String,
    pinned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ProviderStatusStore {
    #[serde(default)]
    asr: BTreeMap<String, ProviderConnectionStatus>,
    #[serde(default)]
    llm: BTreeMap<String, ProviderConnectionStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProviderConnectionStatus {
    tested_at: String,
    ok: bool,
    provider: String,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    message_key: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ConnectionTestRequest {
    #[serde(default)]
    connection_id: Option<String>,
    provider: String,
    endpoint: Option<String>,
    api_key: Option<String>,
    model: Option<String>,
    language: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct ConnectionTestResult {
    ok: bool,
    message: String,
    provider: String,
    tested_at: Option<String>,
    message_key: Option<String>,
}

fn trimmed_option(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn api_key_or_env(value: Option<String>) -> Option<String> {
    trimmed_option(value).or_else(|| {
        std::env::var("OPENAI_API_KEY")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

fn sanitize_connection_error(error: impl std::fmt::Display) -> String {
    let mut message = error.to_string();
    if message.len() > 500 {
        message.truncate(500);
        message.push('…');
    }
    format!("Connection failed: {}", message)
}

fn connection_test_result(
    provider: String,
    result: anyhow::Result<String>,
) -> ConnectionTestResult {
    match result {
        Ok(message) => ConnectionTestResult {
            ok: true,
            message,
            provider,
            tested_at: None,
            message_key: Some("settings.connection_success".into()),
        },
        Err(error) => ConnectionTestResult {
            ok: false,
            message: sanitize_connection_error(error),
            provider,
            tested_at: None,
            message_key: None,
        },
    }
}

fn load_provider_status(path: &Path) -> ProviderStatusStore {
    match std::fs::read_to_string(path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_else(|e| {
            tracing::warn!(
                "failed to parse provider status from {}: {}",
                path.display(),
                e
            );
            ProviderStatusStore::default()
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => ProviderStatusStore::default(),
        Err(e) => {
            tracing::warn!(
                "failed to read provider status from {}: {}",
                path.display(),
                e
            );
            ProviderStatusStore::default()
        }
    }
}

fn save_provider_status(path: &Path, status: &ProviderStatusStore) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(status)?;
    std::fs::write(path, content)?;
    Ok(())
}

fn provider_status_for_kind_mut<'a>(
    store: &'a mut ProviderStatusStore,
    kind: &str,
) -> Option<&'a mut BTreeMap<String, ProviderConnectionStatus>> {
    match kind {
        "asr" => Some(&mut store.asr),
        "llm" => Some(&mut store.llm),
        _ => None,
    }
}

fn provider_status_for_kind<'a>(
    store: &'a ProviderStatusStore,
    kind: &str,
) -> Option<&'a BTreeMap<String, ProviderConnectionStatus>> {
    match kind {
        "asr" => Some(&store.asr),
        "llm" => Some(&store.llm),
        _ => None,
    }
}

fn test_result_to_status(result: &ConnectionTestResult) -> ProviderConnectionStatus {
    ProviderConnectionStatus {
        tested_at: result.tested_at.clone().unwrap_or_default(),
        ok: result.ok,
        provider: result.provider.clone(),
        message: if result.ok {
            None
        } else {
            Some(result.message.clone())
        },
        message_key: result.message_key.clone(),
    }
}

fn asr_connection_test_fields_changed(old: &AsrConnection, new: &AsrConnection) -> bool {
    old.provider != new.provider
        || old.endpoint != new.endpoint
        || old.api_key != new.api_key
        || old.model != new.model
        || old.language != new.language
}

fn llm_connection_test_fields_changed(old: &LlmConnection, new: &LlmConnection) -> bool {
    old.provider != new.provider
        || old.endpoint != new.endpoint
        || old.api_key != new.api_key
        || old.model != new.model
}

fn reset_changed_provider_status(
    old_config: &AppConfig,
    new_config: &AppConfig,
    status: &mut ProviderStatusStore,
) -> bool {
    let mut changed = false;

    for old in &old_config.asr.connections {
        match new_config
            .asr
            .connections
            .iter()
            .find(|new| new.id == old.id)
        {
            Some(new) if asr_connection_test_fields_changed(old, new) => {
                changed |= status.asr.remove(&old.id).is_some();
            }
            None => {
                changed |= status.asr.remove(&old.id).is_some();
            }
            _ => {}
        }
    }

    for old in &old_config.llm.connections {
        match new_config
            .llm
            .connections
            .iter()
            .find(|new| new.id == old.id)
        {
            Some(new) if llm_connection_test_fields_changed(old, new) => {
                changed |= status.llm.remove(&old.id).is_some();
            }
            None => {
                changed |= status.llm.remove(&old.id).is_some();
            }
            _ => {}
        }
    }

    let asr_ids: std::collections::BTreeSet<_> = new_config
        .asr
        .connections
        .iter()
        .map(|connection| connection.id.as_str())
        .collect();
    let before_asr = status.asr.len();
    status.asr.retain(|id, _| asr_ids.contains(id.as_str()));
    changed |= status.asr.len() != before_asr;

    let llm_ids: std::collections::BTreeSet<_> = new_config
        .llm
        .connections
        .iter()
        .map(|connection| connection.id.as_str())
        .collect();
    let before_llm = status.llm.len();
    status.llm.retain(|id, _| llm_ids.contains(id.as_str()));
    changed |= status.llm.len() != before_llm;

    changed
}

fn query_history(conn: &Connection, limit: usize) -> rusqlite::Result<Vec<HistoryEntry>> {
    let sql = if limit > 0 {
        "SELECT id, text, created_at, pinned FROM history ORDER BY pinned DESC, id DESC LIMIT ?1"
    } else {
        "SELECT id, text, created_at, pinned FROM history ORDER BY pinned DESC, id DESC"
    };
    let mut stmt = conn.prepare(sql)?;
    let entries = if limit > 0 {
        stmt.query_map(rusqlite::params![limit], history_entry_from_row)?
            .collect::<Result<Vec<_>, _>>()?
    } else {
        stmt.query_map([], history_entry_from_row)?
            .collect::<Result<Vec<_>, _>>()?
    };
    Ok(entries)
}

fn search_history_query(
    conn: &Connection,
    query: &str,
    limit: usize,
) -> rusqlite::Result<Vec<HistoryEntry>> {
    let pattern = format!("%{}%", query);
    let sql = if limit > 0 {
        "SELECT id, text, created_at, pinned FROM history WHERE text LIKE ?1 ORDER BY pinned DESC, id DESC LIMIT ?2"
    } else {
        "SELECT id, text, created_at, pinned FROM history WHERE text LIKE ?1 ORDER BY pinned DESC, id DESC"
    };
    let mut stmt = conn.prepare(sql)?;
    let entries = if limit > 0 {
        stmt.query_map(rusqlite::params![pattern, limit], history_entry_from_row)?
            .collect::<Result<Vec<_>, _>>()?
    } else {
        stmt.query_map(rusqlite::params![pattern], history_entry_from_row)?
            .collect::<Result<Vec<_>, _>>()?
    };
    Ok(entries)
}

fn history_entry_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<HistoryEntry> {
    Ok(HistoryEntry {
        id: row.get(0)?,
        text: row.get(1)?,
        created_at: row.get(2)?,
        pinned: row.get::<usize, i32>(3)? != 0,
    })
}

fn prune_history(conn: &Connection, limit: usize) {
    if limit > 0 {
        conn.execute(
            "DELETE FROM history WHERE pinned = 0 AND id NOT IN (
                SELECT id FROM history WHERE pinned = 0 ORDER BY id DESC LIMIT ?1
            )",
            rusqlite::params![limit],
        )
        .unwrap_or(0);
    }
}

fn build_pipeline(config: &AppConfig) -> anyhow::Result<Arc<TypeX>> {
    Ok(Arc::new(build_typex_from_config(
        config,
        TypeXBuildOptions::session(),
    )?))
}

// ── Tauri Commands ──

#[tauri::command]
fn list_audio_devices() -> Vec<String> {
    typex_audio::MicrophoneCapture::list_devices().unwrap_or_default()
}

#[tauri::command]
fn handle_record_toggle_cmd(app: tauri::AppHandle) {
    handle_record_toggle(&app);
}

#[derive(Debug, Clone, Serialize)]
struct SystemInfo {
    version: String,
    asr_provider: String,
    asr_model: String,
    audio_device: String,
    injector_method: String,
    plugin_count: usize,
}

#[tauri::command]
fn get_system_info(state: tauri::State<AppState>) -> SystemInfo {
    let config = state.config.lock().unwrap().clone();
    let active_asr = config.asr.active_connection_config();
    SystemInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        asr_provider: active_asr
            .map(|connection| connection.provider.clone())
            .unwrap_or_else(|| config.asr.provider.clone()),
        asr_model: active_asr
            .and_then(|connection| connection.model.clone())
            .unwrap_or_default(),
        audio_device: config
            .audio
            .device
            .clone()
            .unwrap_or_else(|| "default".into()),
        injector_method: config.injector.method.clone(),
        plugin_count: config.pipeline.plugins.len(),
    }
}

#[tauri::command]
fn minimize(window: tauri::WebviewWindow) {
    window.minimize().unwrap();
}

#[tauri::command]
fn close_window(window: tauri::WebviewWindow) {
    window.hide().unwrap();
}

#[tauri::command]
fn get_config(state: tauri::State<AppState>) -> AppConfig {
    state.config.lock().unwrap().clone()
}

#[tauri::command]
fn get_provider_connection_status(
    state: tauri::State<AppState>,
    kind: String,
    connection_id: String,
) -> Option<ProviderConnectionStatus> {
    let store = load_provider_status(&state.provider_status_path);
    provider_status_for_kind(&store, kind.trim())
        .and_then(|statuses| statuses.get(connection_id.trim()).cloned())
}

#[tauri::command]
async fn test_asr_connection(
    app: tauri::AppHandle,
    request: ConnectionTestRequest,
) -> ConnectionTestResult {
    let status_path = provider_status_path(&app);
    let connection_id = request
        .connection_id
        .clone()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| "default".into());
    let provider = request.provider.trim().to_string();
    let result = match provider.as_str() {
        "mock" => Ok("Mock ASR is available.".to_string()),
        "openai-compatible" | "" => test_openai_compatible_asr(request).await,
        other => Err(anyhow::anyhow!("unsupported ASR provider: {}", other)),
    };
    let mut result = connection_test_result(provider, result);
    if result.ok && result.provider == "mock" {
        result.message_key = Some("settings.mock_asr_ok".into());
    }
    result.tested_at = Some(chrono::Utc::now().to_rfc3339());
    save_connection_test_status(&status_path, "asr", &connection_id, &result);
    result
}

#[tauri::command]
async fn test_llm_connection(
    app: tauri::AppHandle,
    request: ConnectionTestRequest,
) -> ConnectionTestResult {
    let status_path = provider_status_path(&app);
    let connection_id = request
        .connection_id
        .clone()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| "default".into());
    let provider = request.provider.trim().to_string();
    let result = match provider.as_str() {
        "mock" | "" => Ok("Mock LLM is available.".to_string()),
        "openai-compatible" => test_openai_compatible_llm(request).await,
        other => Err(anyhow::anyhow!("unsupported LLM provider: {}", other)),
    };
    let mut result = connection_test_result(provider, result);
    if result.ok && result.provider == "mock" {
        result.message_key = Some("settings.mock_llm_ok".into());
    }
    result.tested_at = Some(chrono::Utc::now().to_rfc3339());
    save_connection_test_status(&status_path, "llm", &connection_id, &result);
    result
}

fn save_connection_test_status(
    path: &Path,
    kind: &str,
    connection_id: &str,
    result: &ConnectionTestResult,
) {
    let mut store = load_provider_status(path);
    if let Some(statuses) = provider_status_for_kind_mut(&mut store, kind) {
        statuses.insert(connection_id.to_string(), test_result_to_status(result));
        if let Err(e) = save_provider_status(path, &store) {
            tracing::warn!(
                "failed to save provider status to {}: {}",
                path.display(),
                e
            );
        }
    }
}

async fn test_openai_compatible_asr(request: ConnectionTestRequest) -> anyhow::Result<String> {
    let endpoint =
        trimmed_option(request.endpoint).unwrap_or_else(|| "https://api.openai.com/v1".to_string());
    let api_key = api_key_or_env(request.api_key);
    let model = trimmed_option(request.model).unwrap_or_else(|| "whisper-1".to_string());
    let mut provider = typex_core::typex_asr::openai_compat::OpenAiCompatibleAsrProvider::new(
        endpoint, api_key, model,
    );

    if let Some(language) = trimmed_option(request.language) {
        provider = provider.with_language(language);
    }

    let pcm = vec![0_u8; 16_000 * 2];
    let wav = typex_core::typex_asr::pcm_to_wav(&pcm)?;
    provider
        .transcribe_file(wav, "typex-connection-test.wav")
        .await?;

    Ok("ASR connection OK.".to_string())
}

async fn test_openai_compatible_llm(request: ConnectionTestRequest) -> anyhow::Result<String> {
    let endpoint =
        trimmed_option(request.endpoint).unwrap_or_else(|| "https://api.openai.com/v1".to_string());
    let api_key = api_key_or_env(request.api_key);
    let model = trimmed_option(request.model).unwrap_or_else(|| "gpt-4o-mini".to_string());
    let provider = typex_core::typex_llm::openai_compat::OpenAiCompatibleLlmProvider::new(
        endpoint, api_key, model,
    );

    let input = stream::once(async { Ok("ping".to_string()) }).boxed();
    let mut output = provider.optimize(input);
    while let Some(item) = output.next().await {
        let result = item?;
        if result.is_final || !result.text.trim().is_empty() {
            return Ok("LLM connection OK.".to_string());
        }
    }

    anyhow::bail!("provider returned no response")
}

#[tauri::command]
fn save_config(state: tauri::State<AppState>, mut config: AppConfig) -> Result<(), String> {
    config.normalize_connections_mut();
    let old_config = state.config.lock().unwrap().clone();

    // 1. Persist config to disk FIRST — always save user settings
    config.save(&state.config_path).map_err(|e| e.to_string())?;

    // 2. Reset stale provider test status when connection settings changed
    let mut provider_status = load_provider_status(&state.provider_status_path);
    if reset_changed_provider_status(&old_config, &config, &mut provider_status)
        && let Err(e) = save_provider_status(&state.provider_status_path, &provider_status)
    {
        tracing::warn!(
            "failed to save provider status to {}: {}",
            state.provider_status_path.display(),
            e
        );
    }

    // 3. Update in-memory config immediately
    *state.config.lock().unwrap() = config.clone();

    // 4. Rebuild pipeline (non-fatal: keep old pipeline on failure)
    match build_pipeline(&config) {
        Ok(pipeline) => {
            *state.pipeline.lock().unwrap() = pipeline;
        }
        Err(e) => {
            tracing::warn!("Pipeline rebuild failed (old pipeline kept): {}", e);
        }
    }

    // 5. Update audio capture if not currently recording
    let capture = typex_audio::MicrophoneCapture::new(config.audio.device.clone());
    if state.recording_stop.lock().unwrap().is_none() {
        state.capture.lock().unwrap().replace(capture);
    }

    Ok(())
}

#[tauri::command]
fn get_history(state: tauri::State<AppState>) -> Vec<HistoryEntry> {
    let limit = state.config.lock().unwrap().history.log_limit;
    let db = state.db.lock().unwrap();
    query_history(&db, limit).unwrap_or_default()
}

#[tauri::command]
fn search_history(state: tauri::State<AppState>, query: String) -> Vec<HistoryEntry> {
    let limit = state.config.lock().unwrap().history.log_limit;
    let db = state.db.lock().unwrap();
    search_history_query(&db, &query, limit).unwrap_or_default()
}

#[tauri::command]
fn delete_history_entries(state: tauri::State<AppState>, ids: Vec<i64>) -> Vec<HistoryEntry> {
    let limit = state.config.lock().unwrap().history.log_limit;
    let db = state.db.lock().unwrap();
    for id in &ids {
        db.execute("DELETE FROM history WHERE id = ?1", rusqlite::params![id])
            .unwrap_or(0);
    }
    prune_history(&db, limit);
    query_history(&db, limit).unwrap_or_default()
}

#[tauri::command]
fn toggle_pin_history(state: tauri::State<AppState>, id: i64) -> Vec<HistoryEntry> {
    let limit = state.config.lock().unwrap().history.log_limit;
    let db = state.db.lock().unwrap();
    let current: bool = db
        .query_row(
            "SELECT pinned FROM history WHERE id = ?1",
            rusqlite::params![id],
            |row| row.get::<usize, i32>(0).map(|v| v != 0),
        )
        .unwrap_or(false);
    db.execute(
        "UPDATE history SET pinned = ?1 WHERE id = ?2",
        rusqlite::params![!current as i32, id],
    )
    .unwrap_or(0);
    query_history(&db, limit).unwrap_or_default()
}

#[tauri::command]
fn clear_history(state: tauri::State<AppState>) -> Vec<HistoryEntry> {
    let limit = state.config.lock().unwrap().history.log_limit;
    let db = state.db.lock().unwrap();
    db.execute("DELETE FROM history", []).unwrap_or(0);
    query_history(&db, limit).unwrap_or_default()
}

#[derive(Debug, Clone, Serialize)]
struct RecordingStatePayload {
    recording: bool,
}

#[derive(Debug, Clone, Serialize)]
struct AudioLevelPayload {
    rms: f32,
    peak: f32,
}

/// Overlay state shown in the floating indicator.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)] // Polishing will be used when LLM is wired up
enum OverlayState {
    Recording,
    Transcribing,
    Polishing,
}

fn show_overlay(app: &tauri::AppHandle, state: OverlayState) {
    app.state::<AppState>()
        .overlay_error_token
        .fetch_add(1, Ordering::Relaxed);
    if let Some(window) = app.get_webview_window("overlay") {
        // Only restore position when transitioning from hidden → visible
        if !window.is_visible().unwrap_or(false) {
            let overlay_cfg = app
                .state::<AppState>()
                .config
                .lock()
                .unwrap()
                .overlay
                .clone();
            if let (Some(x), Some(y)) = (overlay_cfg.x, overlay_cfg.y) {
                window
                    .set_position(tauri::Position::Physical(tauri::PhysicalPosition::new(
                        x as i32, y as i32,
                    )))
                    .ok();
            } else if let Some(monitor) = app.primary_monitor().ok().flatten() {
                let screen_width = monitor.size().width as f64;
                let scale = monitor.scale_factor();
                let overlay_width = 300.0;
                let x = (screen_width / scale - overlay_width) / 2.0;
                window
                    .set_position(tauri::Position::Logical(tauri::LogicalPosition::new(
                        x, 20.0,
                    )))
                    .ok();
            }
        }
        window.show().ok();

        // Use eval() instead of emit_to() because the overlay webview
        // does not have __TAURI_INTERNALS__ injected for IPC.
        let state_str = serde_json::to_string(&state).unwrap_or_default();
        let _ = window.eval(format!("window.typexShowOverlay({})", state_str));
    }
}

fn hide_overlay(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("overlay") {
        let _ = window.eval("window.typexHideOverlay()");
        window.hide().ok();
    }
}

/// Show an error message on the overlay. Auto-hides after 5 seconds.
fn show_overlay_error(app: &tauri::AppHandle, message: &str) {
    if let Some(window) = app.get_webview_window("overlay") {
        window.show().ok();
        let json_msg = serde_json::to_string(message).unwrap_or_default();
        let _ = window.eval(format!("window.typexShowError({})", json_msg));

        // Keep the recovery affordance visible long enough to read/copy.
        let app_clone = app.clone();
        let state = app.state::<AppState>();
        let token = state.overlay_error_token.fetch_add(1, Ordering::Relaxed) + 1;
        let rt = state.rt.clone();
        rt.spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(8)).await;
            let current = app_clone
                .state::<AppState>()
                .overlay_error_token
                .load(Ordering::Relaxed);
            if current == token {
                hide_overlay(&app_clone);
            }
        });
    }
}

/// Show successful injection feedback briefly and keep the result copyable while visible.
fn show_overlay_injected(app: &tauri::AppHandle, text: &str) {
    if let Some(window) = app.get_webview_window("overlay") {
        window.show().ok();
        let json_text = serde_json::to_string(text).unwrap_or_default();
        let _ = window.eval(format!("window.typexShowInjected({})", json_text));

        let app_clone = app.clone();
        let state = app.state::<AppState>();
        let token = state.overlay_error_token.fetch_add(1, Ordering::Relaxed) + 1;
        let rt = state.rt.clone();
        rt.spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(2800)).await;
            let current = app_clone
                .state::<AppState>()
                .overlay_error_token
                .load(Ordering::Relaxed);
            if current == token {
                hide_overlay(&app_clone);
            }
        });
    }
}

fn handle_record_toggle(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    let rt = state.rt.clone();
    let mut stop_lock = state.recording_stop.lock().unwrap();

    if let Some(stop_tx) = stop_lock.take() {
        // ── Stop recording ──
        drop(stop_lock);
        // Clear starting flag (no-op if already false).
        state.recording_starting.store(false, Ordering::Release);
        stop_tx.send(()).ok();
        let _ = app.emit(
            "recording-state",
            RecordingStatePayload { recording: false },
        );

        // Show transcribing state in overlay
        show_overlay(app, OverlayState::Transcribing);

        // Spawn async task to collect PCM, transcribe, inject, and save history
        let app_clone = app.clone();
        rt.spawn(async move {
            process_recording(app_clone).await;
        });
    } else {
        // ── Start recording ──
        drop(stop_lock);

        // Guard against concurrent start attempts (the calling thread may be
        // the GUI event loop or a global-shortcut WndProc callback — neither
        // should ever block).
        if state.recording_starting.swap(true, Ordering::AcqRel) {
            tracing::warn!("recording start already in progress, ignoring duplicate toggle");
            return;
        }

        let capture = state.capture.lock().unwrap().take();
        let Some(capture) = capture else {
            state.recording_starting.store(false, Ordering::Release);
            tracing::warn!("no microphone capture available");
            show_overlay_error(app, "麦克风不可用，请检查音频设备设置");
            return;
        };

        // Spawn the entire start-up sequence in a background task so the
        // calling thread returns immediately.
        let app_clone = app.clone();
        rt.spawn(async move {
            let state = app_clone.state::<AppState>();

            let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
            let (level_tx, mut level_rx) =
                tokio::sync::mpsc::channel::<typex_audio::AudioLevel>(32);
            let (started_tx, started_rx) = std::sync::mpsc::channel();

            let acc_future = tokio::task::spawn_blocking(move || {
                let recorder = match capture.record_session_with_levels(Some(level_tx)) {
                    Ok(recorder) => recorder,
                    Err(e) => {
                        let _ = started_tx.send(Err(e.to_string()));
                        return Err(e);
                    }
                };
                let _ = started_tx.send(Ok(()));
                stop_rx.recv().ok();
                Ok(recorder.into_accumulator())
            });

            // Wait for the recording to actually start (or fail).
            // This recv() blocks only this Tokio task, not the GUI thread.
            match started_rx.recv() {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    let new_capture = new_capture_from_state(&state);
                    state.capture.lock().unwrap().replace(new_capture);
                    show_overlay_error(&app_clone, &format!("录音启动失败: {}", e));
                    state.recording_starting.store(false, Ordering::Release);
                    return;
                }
                Err(_) => {
                    let new_capture = new_capture_from_state(&state);
                    state.capture.lock().unwrap().replace(new_capture);
                    show_overlay_error(&app_clone, "录音启动中断");
                    state.recording_starting.store(false, Ordering::Release);
                    return;
                }
            }

            // Recording is now live — publish state.
            state.recording_starting.store(false, Ordering::Release);
            state.recording_stop.lock().unwrap().replace(stop_tx);
            state
                .recording_acc_future
                .lock()
                .unwrap()
                .replace(acc_future);

            // Spawn audio-level monitoring loop.
            let level_app = app_clone.clone();
            tokio::spawn(async move {
                let mut count = 0u32;
                let mut latest: Option<typex_audio::AudioLevel> = None;
                let mut tick = tokio::time::interval(std::time::Duration::from_millis(33));
                loop {
                    tokio::select! {
                        level = level_rx.recv() => {
                            match level {
                                Some(level) => latest = Some(level),
                                None => break,
                            }
                        }
                        _ = tick.tick() => {
                            let Some(level) = latest.take() else { continue; };
                            count += 1;
                            if count <= 20 || count.is_multiple_of(100) {
                                tracing::info!("audio level #{}: rms={:.4}, peak={:.4}", count, level.rms, level.peak);
                            }
                            let rms = finite_level(level.rms);
                            let peak = finite_level(level.peak);
                            let payload = serde_json::json!({ "rms": rms, "peak": peak });

                            // Send to overlay via eval
                            if let Some(w) = level_app.get_webview_window("overlay") {
                                let js = format!("window.typexAudioLevel({})", payload);
                                if let Err(e) = w.eval(&js) && count <= 1 {
                                    tracing::warn!("eval audio-level failed: {}", e);
                                }
                            }

                            // Emit to main window via Tauri event
                            let _ = level_app.emit("audio-level", AudioLevelPayload { rms, peak });
                        }
                    }
                }
                tracing::info!("audio level stream ended after {} forwarded samples", count);
            });

            let _ = app_clone.emit("recording-state", RecordingStatePayload { recording: true });
            show_overlay(&app_clone, OverlayState::Recording);
        });
    }
}

fn finite_level(level: f32) -> f32 {
    if level.is_finite() {
        level.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn new_capture_from_state(state: &AppState) -> typex_audio::MicrophoneCapture {
    typex_audio::MicrophoneCapture::new(state.config.lock().unwrap().audio.device.clone())
}

async fn process_recording(app: tauri::AppHandle) {
    let state = app.state::<AppState>();

    // Step 1: await the spawn_blocking task → get accumulator JoinHandle
    let acc_future = state.recording_acc_future.lock().unwrap().take();
    let acc_future = match acc_future {
        Some(f) => f,
        None => {
            show_overlay_error(&app, "录音状态丢失");
            let _ = app.emit("history-updated", ());
            return;
        }
    };

    // Compute new capture before acquiring the lock to avoid a
    // lock-order inversion with save_config (which holds config → capture).
    let new_capture = new_capture_from_state(&state);
    state.capture.lock().unwrap().replace(new_capture);

    let acc_handle = match acc_future.await {
        Ok(Ok(handle)) => handle,
        Ok(Err(e)) => {
            tracing::error!("recording task failed: {}", e);
            show_overlay_error(&app, &format!("录音任务失败: {}", e));
            let _ = app.emit("history-updated", ());
            return;
        }
        Err(e) => {
            tracing::error!("recording task panicked: {}", e);
            show_overlay_error(&app, &format!("录音任务中断: {}", e));
            let _ = app.emit("history-updated", ());
            return;
        }
    };

    // Step 2: await accumulator → get PCM data
    let pcm = match acc_handle.await {
        Ok(Ok(data)) => data,
        Ok(Err(e)) => {
            tracing::error!("PCM accumulation failed: {}", e);
            show_overlay_error(&app, &format!("音频处理失败: {}", e));
            let _ = app.emit("history-updated", ());
            return;
        }
        Err(e) => {
            tracing::error!("accumulator task panicked: {}", e);
            show_overlay_error(&app, &format!("音频处理中断: {}", e));
            let _ = app.emit("history-updated", ());
            return;
        }
    };

    if pcm.is_empty() {
        tracing::info!("no audio captured");
        show_overlay_error(&app, "未捕获到音频，请检查麦克风权限");
        let _ = app.emit("history-updated", ());
        return;
    }

    tracing::info!("captured {} bytes of PCM audio", pcm.len());

    // Step 3: run pipeline (ASR → plugins → LLM → inject)
    let pipeline = state.pipeline.lock().unwrap().clone();
    let result = match pipeline.run_session(pcm).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("pipeline failed: {}", e);
            show_overlay_error(&app, &format!("转译失败: {}", e));
            let _ = app.emit("history-updated", ());
            return;
        }
    };

    // Step 4: save to history and show completion feedback
    let output_text = result.text.clone();
    if !output_text.trim().is_empty() {
        let now = chrono::Utc::now().to_rfc3339();
        let limit = state.config.lock().unwrap().history.log_limit;
        let db = state.db.lock().unwrap();
        if let Err(e) = db.execute(
            "INSERT INTO history (text, created_at) VALUES (?1, ?2)",
            rusqlite::params![&output_text, now],
        ) {
            tracing::error!("failed to insert history entry: {}", e);
        }
        prune_history(&db, limit);
        drop(db);

        let _ = app.emit("history-updated", ());
        show_overlay_injected(&app, &output_text);
    } else {
        let _ = app.emit("history-updated", ());
        show_overlay_error(&app, "没有识别到可输出文本");
    }
}

#[tauri::command]
fn update_shortcut(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    shortcut: String,
) -> Result<(), String> {
    let gs = app.global_shortcut();
    let old = state.shortcut.lock().unwrap().clone();
    if old == shortcut {
        return Ok(());
    }

    // Register new shortcut first; only unregister old on success.
    // This avoids needing rollback logic if the new shortcut fails.
    if let Err(e) = gs.register(shortcut.as_str()) {
        return Err(format!("快捷键注册失败，可能与其他应用冲突: {}", e));
    }

    if let Err(e) = gs.unregister(old.as_str()) {
        tracing::warn!("failed to unregister old shortcut {}: {}", old, e);
    }

    state.config.lock().unwrap().shortcut.record = shortcut.clone();
    state
        .config
        .lock()
        .unwrap()
        .save(&state.config_path)
        .map_err(|e| e.to_string())?;
    *state.shortcut.lock().unwrap() = shortcut;

    Ok(())
}

#[tauri::command]
fn set_language(state: tauri::State<AppState>, language: String) -> Result<(), String> {
    let mut config = state.config.lock().unwrap();
    config.ui.language = language;
    config.save(&state.config_path).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Initialize tracing subscriber so log messages are visible in console.
    // Uses RUST_LOG env var for filtering (e.g. RUST_LOG=typex_audio=trace,typex_desktop=info).
    // Falls back to "info" level if RUST_LOG is not set.
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(env_filter).init();

    // Create a dedicated Tokio runtime that lives for the entire application
    // lifetime (run() blocks until exit). Its handle is shared with global
    // shortcut callbacks that run outside the async context.
    let runtime = tokio::runtime::Runtime::new().expect("failed to create Tokio runtime");
    let rt = runtime.handle().clone();

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            minimize,
            close_window,
            get_config,
            get_provider_connection_status,
            test_asr_connection,
            test_llm_connection,
            save_config,
            get_history,
            search_history,
            delete_history_entries,
            toggle_pin_history,
            clear_history,
            update_shortcut,
            set_language,
            list_audio_devices,
            handle_record_toggle_cmd,
            get_system_info,
        ])
        .setup(move |app| {
            // Load or create config
            let cfg_path = config_path(app.handle());
            let status_path = provider_status_path(app.handle());
            let config = if cfg_path.exists() {
                AppConfig::load(&cfg_path).unwrap_or_else(|e| {
                    tracing::warn!("failed to load config from {}: {}", cfg_path.display(), e);
                    AppConfig::default()
                })
            } else {
                let config = AppConfig::default();
                if let Err(e) = config.save(&cfg_path) {
                    tracing::warn!("failed to save default config: {}", e);
                }
                config
            };

            let current_shortcut = config.shortcut.record.clone();

            app.handle().plugin(
                tauri_plugin_global_shortcut::Builder::new()
                    .with_handler(move |app, _shortcut, event| {
                        if event.state == ShortcutState::Pressed {
                            handle_record_toggle(app);
                        }
                    })
                    .build(),
            )?;

            // Build pipeline
            let pipeline = build_pipeline(&config).expect("failed to build pipeline");

            // Open database
            let db_file = db_path(app.handle());
            if let Some(parent) = db_file.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            let conn = Connection::open(&db_file).expect("failed to open database");
            init_db(&conn).expect("failed to initialize database");

            let capture = typex_audio::MicrophoneCapture::new(config.audio.device.clone());

            app.manage(AppState {
                config: Mutex::new(config),
                config_path: cfg_path.clone(),
                provider_status_path: status_path,
                db: Mutex::new(conn),
                pipeline: Mutex::new(pipeline),
                capture: Mutex::new(Some(capture)),
                recording_stop: Mutex::new(None),
                recording_acc_future: Mutex::new(None),
                shortcut: Mutex::new(current_shortcut.clone()),
                overlay_error_token: AtomicU64::new(0),
                overlay_save_token: AtomicU64::new(0),
                recording_starting: AtomicBool::new(false),
                rt: rt.clone(),
            });

            if let Err(e) = app.global_shortcut().register(current_shortcut.as_str()) {
                tracing::warn!(
                    "failed to register global shortcut {}: {}",
                    current_shortcut,
                    e
                );
            }

            let show_item = MenuItem::with_id(app, "show", "Show", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let separator = PredefinedMenuItem::separator(app)?;
            let menu = Menu::with_items(app, &[&show_item, &separator, &quit_item])?;

            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .show_menu_on_left_click(false)
                .tooltip("TypeX")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            window.show().unwrap();
                            window.set_focus().unwrap();
                        }
                    }
                    "quit" => {
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let tauri::tray::TrayIconEvent::Click {
                        button,
                        button_state,
                        ..
                    } = event
                        && button == tauri::tray::MouseButton::Left
                        && button_state == tauri::tray::MouseButtonState::Up
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            if window.is_visible().unwrap() {
                                window.hide().unwrap();
                            } else {
                                window.show().unwrap();
                                window.set_focus().unwrap();
                            }
                        }
                    }
                })
                .build(app)?;

            let webview_url = tauri::WebviewUrl::App("index.html".into());
            let window = tauri::WebviewWindowBuilder::new(app, "main", webview_url)
                .title("TypeX")
                .inner_size(920.0, 640.0)
                .min_inner_size(760.0, 520.0)
                .center()
                .visible(false)
                .decorations(false)
                .shadow(true)
                .build()?;

            let window_clone = window.clone();
            window.on_window_event(move |event| {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    window_clone.hide().unwrap();
                }
            });

            // ── Overlay window (always-on-top recording indicator) ──
            let overlay_url = tauri::WebviewUrl::App("overlay.html".into());
            let overlay_window = tauri::WebviewWindowBuilder::new(app, "overlay", overlay_url)
                .title("TypeX Overlay")
                .inner_size(300.0, 66.0)
                .visible(false)
                .decorations(false)
                .transparent(true)
                .shadow(false)
                .always_on_top(true)
                .skip_taskbar(true)
                .build()?;

            // Save overlay position to config whenever the user drags it
            let overlay_app_handle = overlay_window.app_handle().clone();
            let config_path_for_overlay = cfg_path;
            overlay_window.on_window_event(move |event| {
                if let WindowEvent::Moved(position) = event {
                    let state = overlay_app_handle.state::<AppState>();
                    let token = state.overlay_save_token.fetch_add(1, Ordering::Relaxed) + 1;
                    let app_handle = overlay_app_handle.clone();
                    let path = config_path_for_overlay.clone();
                    let rt = state.rt.clone();
                    let x = position.x as f64;
                    let y = position.y as f64;
                    rt.spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                        let state = app_handle.state::<AppState>();
                        if state.overlay_save_token.load(Ordering::Relaxed) == token {
                            let mut config = state.config.lock().unwrap();
                            config.overlay.x = Some(x);
                            config.overlay.y = Some(y);
                            let config_clone = config.clone();
                            drop(config);
                            tokio::task::spawn_blocking(move || {
                                let _ = config_clone.save(&path);
                            });
                        }
                    });
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
