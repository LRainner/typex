use anyhow::Result;
use async_trait::async_trait;
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmResult {
    pub text: String,
    pub is_final: bool,
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn name(&self) -> &str;

    /// Stream raw text in, get optimized text chunks out.
    fn optimize(
        &self,
        text: BoxStream<'static, Result<String>>,
    ) -> BoxStream<'static, Result<LlmResult>>;
}

pub mod mock;
