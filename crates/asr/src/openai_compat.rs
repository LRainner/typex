use anyhow::Result;
use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::{BoxStream, StreamExt};
use serde::Deserialize;
use tracing;

use crate::{AsrProvider, AsrResult};

/// OpenAI-compatible ASR provider (works with OpenAI, Groq, etc.).
///
/// Strategy: buffer incoming PCM audio into fixed-duration segments,
/// encode as WAV, POST to `/v1/audio/transcriptions`, yield text.
pub struct OpenAiCompatibleAsrProvider {
    endpoint: String,
    api_key: String,
    model: String,
    language: Option<String>,
    /// Bytes per segment. Default: 3s of 16kHz 16-bit mono = 96000 bytes.
    segment_bytes: usize,
    client: reqwest::Client,
}

#[derive(Deserialize)]
struct TranscriptionResponse {
    text: String,
}

impl OpenAiCompatibleAsrProvider {
    pub fn new(endpoint: String, api_key: String, model: String) -> Self {
        Self {
            endpoint,
            api_key,
            model,
            language: None,
            segment_bytes: 16000 * 2 * 3, // 16kHz * 2 bytes * 3 seconds
            client: reqwest::Client::new(),
        }
    }

    pub fn with_language(mut self, lang: String) -> Self {
        self.language = Some(lang);
        self
    }

    pub fn with_segment_duration(mut self, seconds: f32) -> Self {
        self.segment_bytes = (16000.0 * 2.0 * seconds) as usize;
        self
    }
}

#[async_trait]
impl AsrProvider for OpenAiCompatibleAsrProvider {
    fn name(&self) -> &str {
        "openai-compatible-asr"
    }

    fn transcribe(&self, audio: BoxStream<'static, Result<Bytes>>) -> BoxStream<'static, Result<AsrResult>> {
        let endpoint = self.endpoint.clone();
        let api_key = self.api_key.clone();
        let model = self.model.clone();
        let language = self.language.clone();
        let segment_bytes = self.segment_bytes;
        let client = self.client.clone();

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<AsrResult>>(8);

        tokio::spawn(async move {
            let mut buffer: Vec<u8> = Vec::new();
            let mut audio = Box::pin(audio);

            while let Some(chunk) = audio.next().await {
                let data = match chunk {
                    Ok(d) => d,
                    Err(e) => {
                        let _ = tx.send(Err(anyhow::anyhow!("audio read error: {}", e))).await;
                        break;
                    }
                };

                buffer.extend_from_slice(&data);

                // Send segment when buffer is full
                while buffer.len() >= segment_bytes {
                    let segment: Vec<u8> = buffer.drain(..segment_bytes).collect();
                    match transcribe_segment(&client, &endpoint, &api_key, &model, &language, &segment).await {
                        Ok(text) => {
                            let _ = tx.send(Ok(AsrResult {
                                text,
                                is_final: false,
                                confidence: 1.0,
                            })).await;
                        }
                        Err(e) => {
                            tracing::warn!("segment transcription failed: {}", e);
                        }
                    }
                }
            }

            // Send remaining audio as final segment
            if !buffer.is_empty() {
                match transcribe_segment(&client, &endpoint, &api_key, &model, &language, &buffer).await {
                    Ok(text) => {
                        let _ = tx.send(Ok(AsrResult {
                            text,
                            is_final: true,
                            confidence: 1.0,
                        })).await;
                    }
                    Err(e) => {
                        let _ = tx.send(Err(anyhow::anyhow!("final segment failed: {}", e))).await;
                    }
                }
            } else {
                // Signal completion even if no remaining audio
                let _ = tx.send(Ok(AsrResult {
                    text: String::new(),
                    is_final: true,
                    confidence: 1.0,
                })).await;
            }
        });

        tokio_stream::wrappers::ReceiverStream::new(rx).boxed()
    }
}

async fn transcribe_segment(
    client: &reqwest::Client,
    endpoint: &str,
    api_key: &str,
    model: &str,
    language: &Option<String>,
    pcm_data: &[u8],
) -> Result<String> {
    let wav = pcm_to_wav(pcm_data);

    let url = format!("{}/audio/transcriptions", endpoint.trim_end_matches('/'));

    let mut form = reqwest::multipart::Form::new()
        .text("model", model.to_string())
        .text("response_format", "json".to_string())
        .part(
            "file",
            reqwest::multipart::Part::bytes(wav)
                .file_name("audio.wav")
                .mime_str("audio/wav")?,
        );

    if let Some(lang) = language {
        form = form.text("language", lang.clone());
    }

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .multipart(form)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("API error {}: {}", status, body);
    }

    let result: TranscriptionResponse = resp.json().await?;
    Ok(result.text.trim().to_string())
}

/// Wrap raw PCM (16kHz, 16-bit, mono) in a WAV header.
fn pcm_to_wav(pcm: &[u8]) -> Vec<u8> {
    let data_len = pcm.len() as u32;
    let file_len = 36 + data_len;

    let mut wav = Vec::with_capacity(44 + pcm.len());
    // RIFF header
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&file_len.to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    // fmt chunk
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());   // chunk size
    wav.extend_from_slice(&1u16.to_le_bytes());    // PCM format
    wav.extend_from_slice(&1u16.to_le_bytes());    // mono
    wav.extend_from_slice(&16000u32.to_le_bytes()); // sample rate
    wav.extend_from_slice(&32000u32.to_le_bytes()); // byte rate (16000 * 2)
    wav.extend_from_slice(&2u16.to_le_bytes());    // block align
    wav.extend_from_slice(&16u16.to_le_bytes());   // bits per sample
    // data chunk
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.extend_from_slice(pcm);

    wav
}
