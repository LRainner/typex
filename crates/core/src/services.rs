use std::sync::Arc;

use tracing::Level;
use typex_asr::AsrProvider;
use typex_config::{AppConfig, AsrConnection, LlmConnection};
use typex_injector::Injector;
use typex_llm::LlmProvider;
use typex_plugin::Plugin;

use crate::{TypeX, TypeXBuildOptions};

pub struct AppServices {
    typex: TypeX,
}

impl AppServices {
    pub fn from_config(config: &AppConfig, options: TypeXBuildOptions) -> anyhow::Result<Self> {
        let asr = ProviderFactory::asr_from_config(config)?;
        let mut builder = TypeX::builder(asr).log_text(config.logging.record_text);

        for name in &config.pipeline.plugins {
            if let Some(plugin) = ProviderFactory::plugin_from_name(name) {
                builder = builder.plugin(plugin);
            } else {
                typex_logging::log_target!(
                    Level::WARN,
                    target: "typex_core",
                    "unknown plugin: {}",
                    name
                );
            }
        }

        if let Some(llm) = ProviderFactory::llm_from_config(config)? {
            builder = builder.llm(llm);
        }

        if let Some(injector) =
            ProviderFactory::injector_from_config(config, options.enable_injector)?
        {
            builder = builder.injector(injector);
        }

        Ok(Self {
            typex: builder.build(),
        })
    }

    pub fn typex(&self) -> &TypeX {
        &self.typex
    }

    pub fn into_typex(self) -> TypeX {
        self.typex
    }
}

pub struct ProviderFactory;

impl ProviderFactory {
    pub fn asr_from_config(config: &AppConfig) -> anyhow::Result<Arc<dyn AsrProvider>> {
        let connection = config
            .asr
            .active_connection_config()
            .ok_or_else(|| anyhow::anyhow!("active ASR connection not found"))?;
        Self::asr_from_connection(connection)
    }

    pub fn asr_from_connection(connection: &AsrConnection) -> anyhow::Result<Arc<dyn AsrProvider>> {
        match connection.provider.trim() {
            "mock" => {
                typex_logging::log_target!(
                    Level::INFO,
                    target: "typex_asr",
                    "ASR provider initialized provider=mock",
                );
                Ok(Arc::new(typex_asr::mock::MockAsrProvider::new()))
            }
            "openai-compatible" | "" => {
                let endpoint = connection
                    .endpoint
                    .as_deref()
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("https://api.openai.com/v1");
                let api_key = connection
                    .api_key
                    .clone()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .or_else(|| {
                        std::env::var("OPENAI_API_KEY")
                            .ok()
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                    });
                let model = connection
                    .model
                    .as_deref()
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("whisper-1");

                let mut provider = typex_asr::openai_compat::OpenAiCompatibleAsrProvider::new(
                    endpoint.to_string(),
                    api_key,
                    model.to_string(),
                );

                if let Some(lang) = connection
                    .language
                    .as_deref()
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                {
                    provider = provider.with_language(lang.to_string());
                }

                typex_logging::log_target!(
                    Level::INFO,
                    target: "typex_asr",
                    format!(
                        "ASR provider initialized provider=openai-compatible model={} endpoint={}",
                        model,
                        typex_logging::redact_url_for_log(endpoint)
                    ),
                );

                Ok(Arc::new(provider))
            }
            other => anyhow::bail!("unknown ASR provider: {}", other),
        }
    }

    pub fn llm_from_config(config: &AppConfig) -> anyhow::Result<Option<Arc<dyn LlmProvider>>> {
        if !config.llm.enabled {
            return Ok(None);
        }

        let connection = config
            .llm
            .active_connection_config()
            .ok_or_else(|| anyhow::anyhow!("active LLM connection not found"))?;
        Self::llm_from_connection(connection, config.llm.prompt.as_deref(), true)
    }

    pub fn llm_from_connection(
        connection: &LlmConnection,
        prompt: Option<&str>,
        enabled: bool,
    ) -> anyhow::Result<Option<Arc<dyn LlmProvider>>> {
        if !enabled {
            return Ok(None);
        }

        match connection.provider.trim() {
            "mock" | "" => {
                typex_logging::log_target!(
                    Level::INFO,
                    target: "typex_llm",
                    "LLM provider initialized provider=mock",
                );
                Ok(Some(Arc::new(typex_llm::mock::MockLlmProvider::new())))
            }
            "openai-compatible" => {
                let endpoint = connection
                    .endpoint
                    .as_deref()
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("https://api.openai.com/v1");
                let api_key = connection
                    .api_key
                    .clone()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .or_else(|| {
                        std::env::var("OPENAI_API_KEY")
                            .ok()
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                    });

                let model = connection
                    .model
                    .as_deref()
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("gpt-4o-mini");

                let provider = typex_llm::openai_compat::OpenAiCompatibleLlmProvider::new(
                    endpoint.to_string(),
                    api_key,
                    model.to_string(),
                )
                .with_system_prompt(prompt.unwrap_or_default().to_string());

                typex_logging::log_target!(
                    Level::INFO,
                    target: "typex_llm",
                    format!(
                        "LLM provider initialized provider=openai-compatible model={} endpoint={}",
                        model,
                        typex_logging::redact_url_for_log(endpoint)
                    ),
                );

                Ok(Some(Arc::new(provider)))
            }
            other => anyhow::bail!("unknown LLM provider: {}", other),
        }
    }

    pub fn injector_from_config(
        config: &AppConfig,
        enabled: bool,
    ) -> anyhow::Result<Option<Arc<dyn Injector>>> {
        if !enabled {
            return Ok(None);
        }

        match config.injector.method.as_str() {
            "clipboard" | "" => Ok(Some(Arc::new(typex_injector::clipboard::ClipboardInjector))),
            "platform" => Ok(Some(Arc::new(typex_injector::platform::PlatformInjector))),
            other => anyhow::bail!("unknown injector method: {}", other),
        }
    }

    pub fn plugin_from_name(name: &str) -> Option<Arc<dyn Plugin>> {
        match name {
            "filler_remover" => Some(Arc::new(typex_plugin::filler_remover::FillerRemover)),
            "sentence_formatter" => Some(Arc::new(
                typex_plugin::sentence_formatter::SentenceFormatter,
            )),
            "text_cleaner" => Some(Arc::new(typex_plugin::text_cleaner::TextCleaner)),
            _ => None,
        }
    }
}
