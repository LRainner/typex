use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    #[serde(default = "default_asr")]
    pub asr: AsrConfig,

    #[serde(default)]
    pub llm: LlmConfig,

    #[serde(default)]
    pub pipeline: PipelineConfig,

    #[serde(default)]
    pub injector: InjectorConfig,

    #[serde(default)]
    pub audio: AudioConfig,

    #[serde(default)]
    pub history: HistoryConfig,

    #[serde(default)]
    pub shortcut: ShortcutConfig,

    #[serde(default)]
    pub overlay: OverlayConfig,

    #[serde(default)]
    pub ui: UiConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsrConfig {
    pub provider: String,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_llm_provider")]
    pub provider: String,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PipelineConfig {
    #[serde(default = "default_performance")]
    pub performance: PerformanceMode,
    #[serde(default)]
    pub plugins: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InjectorConfig {
    #[serde(default = "default_injector")]
    pub method: String,
}

#[derive(Debug, Clone, Default, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PerformanceMode {
    #[default]
    Low,
    Balanced,
    High,
}

impl AppConfig {
    pub fn load(path: &std::path::Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: AppConfig = toml::from_str(&content)?;
        Ok(config)
    }

    pub fn save(&self, path: &std::path::Path) -> Result<()> {
        let content = toml::to_string_pretty(self)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)?;
        Ok(())
    }

    pub fn default_toml() -> String {
        let config = AppConfig::default();
        toml::to_string_pretty(&config).unwrap_or_default()
    }
}

fn default_asr() -> AsrConfig {
    AsrConfig {
        provider: "mock".into(),
        endpoint: None,
        model: None,
        api_key: None,
        language: None,
    }
}

fn default_llm_provider() -> String {
    "mock".into()
}

fn default_performance() -> PerformanceMode {
    PerformanceMode::Balanced
}

fn default_injector() -> String {
    "clipboard".into()
}

impl Default for AsrConfig {
    fn default() -> Self {
        default_asr()
    }
}

impl Default for InjectorConfig {
    fn default() -> Self {
        Self {
            method: default_injector(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AudioConfig {
    #[serde(default)]
    pub device: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryConfig {
    /// Transcription log limit, 0 means unlimited (default)
    #[serde(default)]
    pub log_limit: usize,
    /// Recording session limit, default 50
    #[serde(default = "default_recording_limit")]
    pub recording_limit: usize,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            log_limit: 0,
            recording_limit: default_recording_limit(),
        }
    }
}

fn default_recording_limit() -> usize {
    50
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShortcutConfig {
    /// Record toggle hotkey, e.g. "Ctrl+Alt+Space"
    #[serde(default = "default_shortcut")]
    pub record: String,
}

impl Default for ShortcutConfig {
    fn default() -> Self {
        Self {
            record: default_shortcut(),
        }
    }
}

fn default_shortcut() -> String {
    "Ctrl+Alt+Space".into()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UiConfig {
    /// Language preference: "auto" (follow system), "en", "zh-CN"
    #[serde(default = "default_language")]
    pub language: String,
}

fn default_language() -> String {
    "auto".into()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OverlayConfig {
    /// Overlay window X position (logical pixels), None = auto-center
    #[serde(default)]
    pub x: Option<f64>,
    /// Overlay window Y position (logical pixels), None = default top
    #[serde(default)]
    pub y: Option<f64>,
}
