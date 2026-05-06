use anyhow::Result;
use futures::stream::{BoxStream, StreamExt};
use std::sync::Arc;
use tracing;

use typex_asr::{AsrProvider, AsrResult};
use typex_injector::Injector;
use typex_llm::LlmProvider;
use typex_plugin::{Plugin, PluginContext};

#[derive(Debug, Clone)]
pub struct PipelineOutput {
    pub text: String,
    pub is_final: bool,
}

pub struct Pipeline {
    asr: Arc<dyn AsrProvider>,
    llm: Option<Arc<dyn LlmProvider>>,
    plugins: Vec<Arc<dyn Plugin>>,
    injector: Option<Arc<dyn Injector>>,
}

impl Pipeline {
    pub fn new(asr: Arc<dyn AsrProvider>) -> Self {
        Self {
            asr,
            llm: None,
            plugins: Vec::new(),
            injector: None,
        }
    }

    pub fn with_llm(mut self, llm: Arc<dyn LlmProvider>) -> Self {
        self.llm = Some(llm);
        self
    }

    pub fn with_plugin(mut self, plugin: Arc<dyn Plugin>) -> Self {
        self.plugins.push(plugin);
        self
    }

    pub fn with_injector(mut self, injector: Arc<dyn Injector>) -> Self {
        self.injector = Some(injector);
        self
    }

    /// Run the full streaming pipeline:
    ///   audio → ASR → plugins → LLM (optional) → output
    pub fn run(
        &self,
        audio: BoxStream<'static, Result<bytes::Bytes>>,
    ) -> BoxStream<'static, Result<PipelineOutput>> {
        let asr = self.asr.clone();
        let llm = self.llm.clone();
        let plugins = self.plugins.clone();
        let injector = self.injector.clone();

        // Stage 1: ASR
        let asr_stream = asr.transcribe(audio);

        // Stage 2: Apply plugins to each ASR result
        let plugin_stream = asr_stream.then(move |result| {
            let plugins = plugins.clone();
            let injector = injector.clone();
            async move {
                let asr_result = result?;

                let text = apply_plugins(&asr_result.text, &asr_result, &plugins).await?;

                // Inject into system if injector is set
                if let Some(ref inj) = injector {
                    if let Err(e) = inj.inject(&text) {
                        tracing::warn!("injector failed: {}", e);
                    }
                }

                Ok(PipelineOutput {
                    text,
                    is_final: asr_result.is_final,
                })
            }
        });

        // Stage 3: LLM optimization (optional)
        if let Some(llm) = llm {
            let llm_stream = Self::attach_llm(plugin_stream, llm);
            llm_stream.boxed()
        } else {
            plugin_stream.boxed()
        }
    }

    fn attach_llm(
        input: impl futures::Stream<Item = Result<PipelineOutput>> + Send + 'static,
        llm: Arc<dyn LlmProvider>,
    ) -> BoxStream<'static, Result<PipelineOutput>> {
        // Collect final chunks, send to LLM as a stream, and yield LLM output
        let (tx, rx) = tokio::sync::mpsc::channel::<String>(32);

        // Forward final text chunks to LLM input channel
        let forwarder = input.for_each(move |item| {
            let tx = tx.clone();
            async move {
                if let Ok(output) = item {
                    let _ = tx.send(output.text).await;
                }
            }
        });

        // Create LLM input stream from channel
        let llm_input = tokio_stream::wrappers::ReceiverStream::new(rx)
            .map(|s| Ok(s))
            .boxed();

        let llm_output = llm.optimize(llm_input);

        // Merge: yield original non-final chunks, then LLM results
        // For simplicity, just return the LLM stream
        let merged = llm_output.map(|r| {
            let lr = r?;
            Ok(PipelineOutput {
                text: lr.text,
                is_final: lr.is_final,
            })
        });

        // Spawn the forwarder in background
        tokio::spawn(forwarder);

        merged.boxed()
    }
}

async fn apply_plugins(
    text: &str,
    asr_result: &AsrResult,
    plugins: &[Arc<dyn Plugin>],
) -> Result<String> {
    let mut result = text.to_string();
    let ctx = PluginContext {
        is_final: asr_result.is_final,
    };
    for plugin in plugins {
        result = plugin.process(&result, &ctx).await?;
    }
    Ok(result)
}
