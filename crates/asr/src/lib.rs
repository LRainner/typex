use anyhow::Result;
use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt;
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
    fn transcribe(
        &self,
        audio: BoxStream<'static, Result<Bytes>>,
    ) -> BoxStream<'static, Result<AsrResult>>;

    /// Transcribe a complete audio file (any format the API supports).
    /// Default: wrap bytes in a single-shot stream, call transcribe(), collect final result.
    async fn transcribe_file(&self, data: Vec<u8>, _filename: &str) -> Result<AsrResult> {
        let bytes = Bytes::from(data);
        let stream = futures::stream::once(async move { Ok(bytes) }).boxed();
        let mut results = Box::pin(self.transcribe(stream));
        let mut final_text = String::new();
        while let Some(result) = results.next().await {
            match result {
                Ok(r) => {
                    let chunk = r.text.trim();
                    if !chunk.is_empty() {
                        if !final_text.is_empty() {
                            final_text.push(' ');
                        }
                        final_text.push_str(chunk);
                    }
                    if r.is_final {
                        break;
                    }
                }
                Err(e) => return Err(e),
            }
        }
        Ok(AsrResult {
            text: final_text,
            is_final: true,
            confidence: 1.0,
        })
    }
}

/// Wrap raw 16kHz 16-bit mono PCM in a WAV header.
pub fn pcm_to_wav(pcm: &[u8]) -> Result<Vec<u8>> {
    use std::io::Cursor;

    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(&mut cursor, spec)
            .map_err(|e| anyhow::anyhow!("wav writer init failed: {}", e))?;
        if !pcm.len().is_multiple_of(2) {
            tracing::warn!(
                "odd-length PCM data ({} bytes), dropping last byte",
                pcm.len()
            );
        }
        for chunk in pcm.chunks_exact(2) {
            writer
                .write_sample(i16::from_le_bytes([chunk[0], chunk[1]]))
                .map_err(|e| anyhow::anyhow!("wav write sample failed: {}", e))?;
        }
        writer
            .finalize()
            .map_err(|e| anyhow::anyhow!("wav finalize failed: {}", e))?;
    }
    Ok(cursor.into_inner())
}

pub mod mock;
pub mod openai_compat;
