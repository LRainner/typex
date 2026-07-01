pub use typex_asr;
pub use typex_audio;
pub use typex_config;
pub use typex_injector;
pub use typex_llm;
pub use typex_pipeline;
pub use typex_plugin;
pub use typex_provider;

mod services;

pub use services::{AppServices, ProviderFactory};
pub use typex_provider::{ProviderError, ProviderErrorKind, ProviderService, find_provider_error};

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
    AppServices::from_config(config, options).map(AppServices::into_typex)
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
