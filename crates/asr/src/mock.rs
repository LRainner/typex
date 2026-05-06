use anyhow::Result;
use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::{BoxStream, StreamExt};
use tokio::time::{interval, Duration};
use tokio_stream::wrappers::IntervalStream;

use crate::{AsrProvider, AsrResult};

pub struct MockAsrProvider {
    chunks: Vec<&'static str>,
}

impl MockAsrProvider {
    pub fn new() -> Self {
        Self {
            chunks: vec![
                "你好",
                "你好，这个",
                "你好，这个是一个",
                "你好，这是一个测试",
            ],
        }
    }
}

impl Default for MockAsrProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AsrProvider for MockAsrProvider {
    fn name(&self) -> &str {
        "mock-asr"
    }

    fn transcribe(&self, _audio: BoxStream<'static, Result<Bytes>>) -> BoxStream<'static, Result<AsrResult>> {
        let chunks = self.chunks.clone();
        let count = chunks.len();
        let ticker = IntervalStream::new(interval(Duration::from_millis(300)));

        let stream = ticker
            .enumerate()
            .map(move |(i, _)| {
                let is_final = i == count - 1;
                Ok(AsrResult {
                    text: chunks[i].to_string(),
                    is_final,
                    confidence: 0.95,
                })
            })
            .take(count);

        stream.boxed()
    }
}
