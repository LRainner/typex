pub use typex_asr;
pub use typex_audio;
pub use typex_config;
pub use typex_injector;
pub use typex_llm;
pub use typex_pipeline;
pub use typex_plugin;

use std::sync::Arc;
use typex_asr::AsrProvider;
use typex_injector::Injector;
use typex_llm::LlmProvider;
use typex_pipeline::Pipeline;
use typex_plugin::Plugin;

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
