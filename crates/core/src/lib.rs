pub use typex_asr;
pub use typex_audio;
pub use typex_config;
pub use typex_injector;
pub use typex_llm;
pub use typex_pipeline;
pub use typex_plugin;

use std::sync::Arc;
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
    let mut builder = TypeX::builder(asr);

    for name in &config.pipeline.plugins {
        if let Some(plugin) = create_plugin(name) {
            builder = builder.plugin(plugin);
        } else {
            tracing::warn!("unknown plugin: {}", name);
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
    match config.asr.provider.as_str() {
        "mock" => Ok(Arc::new(typex_asr::mock::MockAsrProvider::new())),
        "openai-compatible" | "" => {
            let endpoint = config
                .asr
                .endpoint
                .as_deref()
                .unwrap_or("https://api.openai.com/v1");
            let api_key = config
                .asr
                .api_key
                .clone()
                .or_else(|| std::env::var("OPENAI_API_KEY").ok())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let model = config.asr.model.as_deref().unwrap_or("whisper-1");

            let mut provider = typex_asr::openai_compat::OpenAiCompatibleAsrProvider::new(
                endpoint.to_string(),
                api_key,
                model.to_string(),
            );

            if let Some(lang) = &config.asr.language {
                provider = provider.with_language(lang.clone());
            }

            Ok(Arc::new(provider))
        }
        other => anyhow::bail!("unknown ASR provider: {}", other),
    }
}

fn create_llm_provider(config: &AppConfig) -> anyhow::Result<Option<Arc<dyn LlmProvider>>> {
    if !config.llm.enabled {
        return Ok(None);
    }

    match config.llm.provider.as_str() {
        "mock" | "" => Ok(Some(Arc::new(typex_llm::mock::MockLlmProvider::new()))),
        "openai-compatible" => {
            let endpoint = config
                .llm
                .endpoint
                .as_deref()
                .unwrap_or("https://api.openai.com/v1");
            let api_key = config
                .llm
                .api_key
                .clone()
                .or_else(|| std::env::var("OPENAI_API_KEY").ok())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());

            let model = config.llm.model.as_deref().unwrap_or("gpt-4o-mini");

            let has_key = api_key.is_some();
            tracing::info!(
                "LLM provider={} model={} has_api_key={}",
                "openai-compatible",
                model,
                has_key
            );
            let provider = typex_llm::openai_compat::OpenAiCompatibleLlmProvider::new(
                endpoint.to_string(),
                api_key,
                model.to_string(),
            )
            .with_system_prompt(config.llm.prompt.clone().unwrap_or_default());

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
}

impl TypeX {
    pub fn builder(asr: Arc<dyn AsrProvider>) -> TypeXBuilder {
        TypeXBuilder {
            pipeline: Pipeline::new(asr),
        }
    }

    pub fn pipeline(&self) -> &Pipeline {
        &self.pipeline
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

    pub fn build(self) -> TypeX {
        TypeX {
            pipeline: self.pipeline,
        }
    }
}
