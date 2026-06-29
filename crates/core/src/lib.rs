pub use typex_asr;
pub use typex_audio;
pub use typex_config;
pub use typex_injector;
pub use typex_llm;
pub use typex_pipeline;
pub use typex_plugin;

use std::sync::Arc;
use tracing::Level;
use typex_asr::AsrProvider;
use typex_config::AppConfig;
use typex_injector::Injector;
use typex_llm::LlmProvider;
use typex_pipeline::Pipeline;
use typex_plugin::Plugin;

#[derive(Debug, Clone, Copy)]
pub struct TypeXBuildOptions {
    pub enable_injector: bool,
}

impl TypeXBuildOptions {
    pub fn session() -> Self {
        Self {
            enable_injector: true,
        }
    }

    pub fn file() -> Self {
        Self {
            enable_injector: false,
        }
    }
}

pub fn build_typex_from_config(
    config: &AppConfig,
    options: TypeXBuildOptions,
) -> anyhow::Result<TypeX> {
    let asr = create_asr_provider(config)?;
    let mut builder = TypeX::builder(asr).log_text(config.logging.record_text);

    for name in &config.pipeline.plugins {
        if let Some(plugin) = create_plugin(name) {
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

    if let Some(llm) = create_llm_provider(config)? {
        builder = builder.llm(llm);
    }

    if let Some(injector) = create_injector(config, options.enable_injector)? {
        builder = builder.injector(injector);
    }

    Ok(builder.build())
}

fn create_asr_provider(config: &AppConfig) -> anyhow::Result<Arc<dyn AsrProvider>> {
    let connection = config
        .asr
        .active_connection_config()
        .ok_or_else(|| anyhow::anyhow!("active ASR connection not found"))?;

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

fn create_llm_provider(config: &AppConfig) -> anyhow::Result<Option<Arc<dyn LlmProvider>>> {
    if !config.llm.enabled {
        return Ok(None);
    }

    let connection = config
        .llm
        .active_connection_config()
        .ok_or_else(|| anyhow::anyhow!("active LLM connection not found"))?;

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
            .with_system_prompt(config.llm.prompt.clone().unwrap_or_default());

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

fn create_injector(config: &AppConfig, enabled: bool) -> anyhow::Result<Option<Arc<dyn Injector>>> {
    if !enabled {
        return Ok(None);
    }

    match config.injector.method.as_str() {
        "clipboard" | "" => Ok(Some(Arc::new(typex_injector::clipboard::ClipboardInjector))),
        "platform" => Ok(Some(Arc::new(typex_injector::platform::PlatformInjector))),
        other => anyhow::bail!("unknown injector method: {}", other),
    }
}

fn create_plugin(name: &str) -> Option<Arc<dyn Plugin>> {
    match name {
        "filler_remover" => Some(Arc::new(typex_plugin::filler_remover::FillerRemover)),
        "sentence_formatter" => Some(Arc::new(
            typex_plugin::sentence_formatter::SentenceFormatter,
        )),
        "text_cleaner" => Some(Arc::new(typex_plugin::text_cleaner::TextCleaner)),
        _ => None,
    }
}

/// Convenience builder for a fully-wired Pipeline.
pub struct TypeX {
    pipeline: Pipeline,
    log_text: bool,
}

impl TypeX {
    pub fn builder(asr: Arc<dyn AsrProvider>) -> TypeXBuilder {
        TypeXBuilder {
            pipeline: Pipeline::new(asr),
            log_text: false,
        }
    }

    pub fn pipeline(&self) -> &Pipeline {
        &self.pipeline
    }

    pub fn log_text(&self) -> bool {
        self.log_text
    }

    pub async fn run_session(
        &self,
        pcm_data: Vec<u8>,
    ) -> anyhow::Result<typex_pipeline::PipelineOutput> {
        self.pipeline.run_session(pcm_data).await
    }

    pub async fn run_file(
        &self,
        file_data: Vec<u8>,
        filename: &str,
    ) -> anyhow::Result<typex_pipeline::PipelineOutput> {
        self.pipeline.run_file(file_data, filename).await
    }
}

pub struct TypeXBuilder {
    pipeline: Pipeline,
    log_text: bool,
}

impl TypeXBuilder {
    pub fn llm(mut self, provider: Arc<dyn LlmProvider>) -> Self {
        self.pipeline = self.pipeline.with_llm(provider);
        self
    }

    pub fn plugin(mut self, plugin: Arc<dyn Plugin>) -> Self {
        self.pipeline = self.pipeline.with_plugin(plugin);
        self
    }

    pub fn injector(mut self, injector: Arc<dyn Injector>) -> Self {
        self.pipeline = self.pipeline.with_injector(injector);
        self
    }

    pub fn log_text(mut self, log_text: bool) -> Self {
        self.pipeline = self.pipeline.with_log_text(log_text);
        self.log_text = log_text;
        self
    }

    pub fn build(self) -> TypeX {
        TypeX {
            pipeline: self.pipeline,
            log_text: self.log_text,
        }
    }
}
