use anyhow::Result;
use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::{BoxStream, StreamExt};
use serde::Deserialize;
use crate::{AsrProvider, AsrResult};

// PCM format: 16kHz, 16-bit, mono — Whisper API standard
const SAMPLE_RATE: u32 = 16000;
const BITS_PER_SAMPLE: u16 = 16;
const CHANNELS: u16 = 1;
const BYTES_PER_SAMPLE: u16 = BITS_PER_SAMPLE / 8; // 2 bytes per sample

/// OpenAI-compatible ASR provider (works with OpenAI, etc.).
///
/// Strategy: buffer incoming PCM audio into fixed-duration segments,
/// encode as WAV, POST to `/v1/audio/transcriptions`, yield text.
pub struct OpenAiCompatibleAsrProvider {
    endpoint: String,
    api_key: String,
    model: String,
    language: Option<String>,
    /// Bytes per segment. Default: 3s of 16kHz 16-bit mono.
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
            segment_bytes: (SAMPLE_RATE as usize) * (BYTES_PER_SAMPLE as usize) * (CHANNELS as usize) * 3,
            client: reqwest::Client::new(),
        }
    }

    pub fn with_language(mut self, lang: String) -> Self {
        self.language = Some(lang);
        self
    }

    pub fn with_segment_duration(mut self, seconds: f32) -> Self {
        self.segment_bytes = (SAMPLE_RATE as f32 * BYTES_PER_SAMPLE as f32 * CHANNELS as f32 * seconds.max(0.1)) as usize;
        self
    }

    /// Transcribe a complete audio file (any format the API supports: wav, mp3, ogg, etc.).
    /// Sends the file as-is — no format conversion needed.
    pub async fn transcribe_file(&self, file_data: &[u8], filename: &str) -> Result<AsrResult> {
        let url = format!("{}/audio/transcriptions", self.endpoint.trim_end_matches('/'));

        let mime = guess_mime(filename);
        let mut form = reqwest::multipart::Form::new()
            .text("model", self.model.clone())
            .text("response_format", "json".to_string())
            .part(
                "file",
                reqwest::multipart::Part::bytes(file_data.to_vec())
                    .file_name(filename.to_string())
                    .mime_str(mime)?,
            );

        if let Some(lang) = &self.language {
            form = form.text("language", lang.clone());
        }

        let resp = self.client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .multipart(form)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("API error {}: {}", status, body);
        }

        let result: TranscriptionResponse = resp.json().await?;
        Ok(AsrResult {
            text: result.text.trim().to_string(),
            is_final: true,
            confidence: 1.0,
        })
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
            let mut ok = true;

            while ok && let Some(chunk) = audio.next().await {
                let data = match chunk {
                    Ok(d) => d,
                    Err(e) => {
                        let _ = tx.send(Err(e.context("audio read error"))).await;
                        break;
                    }
                };

                buffer.extend_from_slice(&data);

                while buffer.len() >= segment_bytes {
                    let segment: Vec<u8> = buffer.drain(..segment_bytes).collect();
                    match transcribe_segment(&client, &endpoint, &api_key, &model, &language, &segment).await {
                        Ok(text) => {
                            if tx.send(Ok(AsrResult {
                                text,
                                is_final: false,
                                confidence: 1.0,
                            })).await.is_err() {
                                return;
                            }
                        }
                        Err(e) => {
                            let _ = tx.send(Err(e.context("segment transcription failed"))).await;
                            ok = false;
                        }
                    }
                }
            }

            if ok && !buffer.is_empty() {
                match transcribe_segment(&client, &endpoint, &api_key, &model, &language, &buffer).await {
                    Ok(text) => {
                        let _ = tx.send(Ok(AsrResult {
                            text,
                            is_final: true,
                            confidence: 1.0,
                        })).await;
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e.context("final segment failed"))).await;
                    }
                }
            } else if ok {
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

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("API error {}: {}", status, body);
    }

    let result: TranscriptionResponse = resp.json().await?;
    Ok(result.text.trim().to_string())
}

/// Wrap raw PCM in a WAV header using hound.
fn pcm_to_wav(pcm: &[u8]) -> Vec<u8> {
    use std::io::Cursor;

    let spec = hound::WavSpec {
        channels: CHANNELS,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: BITS_PER_SAMPLE,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(&mut cursor, spec).unwrap();
        let samples = pcm.chunks(2).map(|c| i16::from_le_bytes([c[0], c[1]]));
        for sample in samples {
            writer.write_sample(sample).unwrap();
        }
        writer.finalize().unwrap();
    }
    cursor.into_inner()
}

fn guess_mime(filename: &str) -> &'static str {
    match filename.rsplit('.').next() {
        Some("mp3") => "audio/mpeg",
        Some("ogg") => "audio/ogg",
        Some("flac") => "audio/flac",
        Some("m4a") => "audio/mp4",
        Some("webm") => "audio/webm",
        _ => "audio/wav",
    }
}
