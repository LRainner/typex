use anyhow::Result;
use async_trait::async_trait;
use futures::stream::{BoxStream, StreamExt};

use crate::{LlmProvider, LlmResult};

pub struct MockLlmProvider;

impl MockLlmProvider {
    pub fn new() -> Self {
        Self
    }
}

impl Default for MockLlmProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl LlmProvider for MockLlmProvider {
    fn name(&self) -> &str {
        "mock-llm"
    }

    fn optimize(
        &self,
        text: BoxStream<'static, Result<String>>,
    ) -> BoxStream<'static, Result<LlmResult>> {
        let stream = text.enumerate().map(|(i, chunk)| {
            let t = chunk?;
            let is_final = false; // In a real impl, detect end-of-stream
            Ok(LlmResult {
                text: format!("{} [optimized]", t),
                is_final: is_final && i == 0, // simplification
            })
        });

        stream.boxed()
    }
}
