use anyhow::Result;
use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsrResult {
    pub text: String,
    pub is_final: bool,
    pub confidence: f32,
}

#[async_trait]
pub trait AsrProvider: Send + Sync {
    fn name(&self) -> &str;

    /// Stream audio in, get text chunks out.
    /// The implementation owns the audio ingestion loop and yields
    /// partial/final results as they become available.
    fn transcribe(&self, audio: BoxStream<'static, Result<Bytes>>)
        -> BoxStream<'static, Result<AsrResult>>;
}

pub mod mock;
pub mod openai_compat;
