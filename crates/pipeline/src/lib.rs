use anyhow::Result;
use futures::stream::{BoxStream, StreamExt};
use std::sync::Arc;

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

    pub fn run(
        &self,
        audio: BoxStream<'static, Result<bytes::Bytes>>,
    ) -> BoxStream<'static, Result<PipelineOutput>> {
        let asr = self.asr.clone();
        let llm = self.llm.clone();
        let plugins = self.plugins.clone();
        let injector = self.injector.clone();

        let asr_stream = asr.transcribe(audio);

        let plugin_stream = asr_stream.then(move |result| {
            let plugins = plugins.clone();
            let injector = injector.clone();
            async move {
                let asr_result = result?;

                let text = apply_plugins(&asr_result.text, &asr_result, &plugins).await?;

                if let Some(ref inj) = injector
                    && let Err(e) = inj.inject(&text)
                {
                    tracing::warn!("injector failed: {}", e);
                }

                Ok(PipelineOutput {
                    text,
                    is_final: asr_result.is_final,
                })
            }
        });

        if let Some(llm) = llm {
            let llm_stream = Self::attach_llm(plugin_stream, llm);
            llm_stream.boxed()
        } else {
            plugin_stream.boxed()
        }
    }

    pub async fn run_session(&self, pcm_data: Vec<u8>) -> Result<PipelineOutput> {
        let wav = typex_asr::pcm_to_wav(&pcm_data)?;
        let asr_result = self.asr.transcribe_file(wav, "audio.wav").await?;

        let text = apply_plugins(&asr_result.text, &asr_result, &self.plugins).await?;

        let final_text = match &self.llm {
            Some(llm) => {
                let input = futures::stream::once(async move { Ok(text) }).boxed();
                let mut output = llm.optimize(input);
                let mut optimized = String::new();
                while let Some(res) = output.next().await {
                    let chunk = res?.text.trim().to_string();
                    if !chunk.is_empty() {
                        if !optimized.is_empty() {
                            optimized.push(' ');
                        }
                        optimized.push_str(&chunk);
                    }
                }
                optimized
            }
            None => text,
        };

        if let Some(ref inj) = self.injector
            && let Err(e) = inj.inject(&final_text)
        {
            tracing::warn!("injector failed: {}", e);
        }

        Ok(PipelineOutput {
            text: final_text,
            is_final: true,
        })
    }

    fn attach_llm(
        input: impl futures::Stream<Item = Result<PipelineOutput>> + Send + 'static,
        llm: Arc<dyn LlmProvider>,
    ) -> BoxStream<'static, Result<PipelineOutput>> {
        let (tx, rx) = tokio::sync::mpsc::channel::<String>(32);

        let forwarder = input.for_each(move |item| {
            let tx = tx.clone();
            async move {
                if let Ok(output) = item {
                    let _ = tx.send(output.text).await;
                }
            }
        });

        let llm_input = tokio_stream::wrappers::ReceiverStream::new(rx)
            .map(Ok)
            .boxed();

        let llm_output = llm.optimize(llm_input);

        let merged = llm_output.map(|r| {
            let lr = r?;
            Ok(PipelineOutput {
                text: lr.text,
                is_final: lr.is_final,
            })
        });

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
