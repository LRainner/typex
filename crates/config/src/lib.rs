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

    #[serde(default)]
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsrConfig {
    #[serde(default = "default_asr_provider")]
    pub provider: String,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default = "default_connection_id")]
    pub active_connection: String,
    #[serde(default)]
    pub connections: Vec<AsrConnection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsrConnection {
    pub id: String,
    pub name: String,
    #[serde(default = "default_asr_provider")]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    #[serde(default = "default_connection_id")]
    pub active_connection: String,
    #[serde(default)]
    pub connections: Vec<LlmConnection>,
    /// Custom system prompt for text optimization.
    /// If None or empty, a sensible default is used.
    #[serde(default)]
    pub prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConnection {
    pub id: String,
    pub name: String,
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
        let mut config: AppConfig = toml::from_str(&content)?;
        config.normalize_connections_mut();
        Ok(config)
    }

    pub fn save(&self, path: &std::path::Path) -> Result<()> {
        let mut config = self.clone();
        config.normalize_connections_mut();
        let content = toml::to_string_pretty(&config)?;
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

    pub fn normalize_connections_mut(&mut self) {
        self.asr.normalize_connections_mut();
        self.llm.normalize_connections_mut();
    }
}

impl AsrConfig {
    pub fn normalize_connections_mut(&mut self) {
        if self.active_connection.trim().is_empty() {
            self.active_connection = default_connection_id();
        }

        if self.connections.is_empty() {
            self.connections.push(AsrConnection {
                id: self.active_connection.clone(),
                name: "Default ASR".into(),
                provider: if self.provider.trim().is_empty() {
                    default_asr_provider()
                } else {
                    self.provider.clone()
                },
                endpoint: self.endpoint.clone(),
                model: self.model.clone(),
                api_key: self.api_key.clone(),
                language: self.language.clone(),
            });
        }

        if !self
            .connections
            .iter()
            .any(|connection| connection.id == self.active_connection)
            && let Some(first) = self.connections.first()
        {
            self.active_connection = first.id.clone();
        }

        if let Some(active) = self.active_connection_config().cloned() {
            self.provider = active.provider;
            self.endpoint = active.endpoint;
            self.model = active.model;
            self.api_key = active.api_key;
            self.language = active.language;
        }
    }

    pub fn active_connection_config(&self) -> Option<&AsrConnection> {
        self.connections
            .iter()
            .find(|connection| connection.id == self.active_connection)
            .or_else(|| self.connections.first())
    }
}

impl LlmConfig {
    pub fn normalize_connections_mut(&mut self) {
        if self.active_connection.trim().is_empty() {
            self.active_connection = default_connection_id();
        }

        if self.connections.is_empty() {
            self.connections.push(LlmConnection {
                id: self.active_connection.clone(),
                name: "Default LLM".into(),
                provider: if self.provider.trim().is_empty() {
                    default_llm_provider()
                } else {
                    self.provider.clone()
                },
                endpoint: self.endpoint.clone(),
                model: self.model.clone(),
                api_key: self.api_key.clone(),
            });
        }

        if !self
            .connections
            .iter()
            .any(|connection| connection.id == self.active_connection)
            && let Some(first) = self.connections.first()
        {
            self.active_connection = first.id.clone();
        }

        if let Some(active) = self.active_connection_config().cloned() {
            self.provider = active.provider;
            self.endpoint = active.endpoint;
            self.model = active.model;
            self.api_key = active.api_key;
        }
    }

    pub fn active_connection_config(&self) -> Option<&LlmConnection> {
        self.connections
            .iter()
            .find(|connection| connection.id == self.active_connection)
            .or_else(|| self.connections.first())
    }
}

fn default_asr() -> AsrConfig {
    let mut config = AsrConfig {
        provider: default_asr_provider(),
        endpoint: None,
        model: None,
        api_key: None,
        language: None,
        active_connection: default_connection_id(),
        connections: Vec::new(),
    };
    config.normalize_connections_mut();
    config
}

fn default_asr_provider() -> String {
    "mock".into()
}

fn default_llm_provider() -> String {
    "mock".into()
}

fn default_connection_id() -> String {
    "default".into()
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

impl Default for LlmConfig {
    fn default() -> Self {
        let mut config = Self {
            enabled: false,
            provider: default_llm_provider(),
            endpoint: None,
            model: None,
            api_key: None,
            active_connection: default_connection_id(),
            connections: Vec::new(),
            prompt: None,
        };
        config.normalize_connections_mut();
        config
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
    /// Theme preference: "system" (follow OS), "light", "dark"
    #[serde(default = "default_theme")]
    pub theme: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    #[serde(default)]
    pub level: LogLevel,
    #[serde(default)]
    pub record_text: bool,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: LogLevel::Info,
            record_text: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Error,
    Warn,
    #[default]
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Trace => "trace",
        }
    }
}

fn default_language() -> String {
    "auto".into()
}

fn default_theme() -> String {
    "system".into()
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

#[cfg(test)]
mod tests;
