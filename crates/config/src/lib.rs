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
