use crate::{AsrProvider, AsrResult};
use anyhow::Result;
use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::{BoxStream, StreamExt};
use serde::Deserialize;
use typex_provider::{ProviderError, ProviderErrorKind, ProviderService, kind_from_http_status};

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
    api_key: Option<String>,
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
    pub fn new(endpoint: String, api_key: Option<String>, model: String) -> Self {
        Self {
            endpoint,
            api_key,
            model,
            language: None,
            segment_bytes: (SAMPLE_RATE as usize)
                * (BYTES_PER_SAMPLE as usize)
                * (CHANNELS as usize)
                * 3,
            client: reqwest::Client::new(),
        }
    }

    pub fn with_language(mut self, lang: String) -> Self {
        self.language = Some(lang);
        self
    }

    pub fn with_segment_duration(mut self, seconds: f32) -> Self {
        self.segment_bytes =
            (SAMPLE_RATE as f32 * BYTES_PER_SAMPLE as f32 * CHANNELS as f32 * seconds.max(0.1))
                as usize;
        self
    }
}

#[async_trait]
impl AsrProvider for OpenAiCompatibleAsrProvider {
    fn name(&self) -> &str {
        "openai-compatible-asr"
    }

    /// Transcribe a complete audio file (any format the API supports: wav, mp3, ogg, etc.).
    /// Sends the file as-is — no format conversion needed.
    async fn transcribe_file(&self, file_data: Vec<u8>, filename: &str) -> Result<AsrResult> {
        let url = transcription_url(&self.endpoint);
        let mime = guess_mime(filename);
        let mut form = reqwest::multipart::Form::new()
            .text("model", self.model.clone())
            .text("response_format", "json".to_string())
            .part(
                "file",
                reqwest::multipart::Part::bytes(file_data)
                    .file_name(filename.to_string())
                    .mime_str(mime)?,
            );

        if let Some(lang) = &self.language {
            form = form.text("language", lang.clone());
        }

        typex_logging::log_target!(
            tracing::Level::DEBUG,
            target: "typex_asr",
            format!(
                "ASR transcription request started filename={} model={} endpoint={}",
                filename, self.model, self.endpoint
            ),
        );
        let text = post_transcription(&self.client, &url, self.api_key.as_deref(), form).await?;
        typex_logging::log_target!(
            tracing::Level::DEBUG,
            target: "typex_asr",
            "ASR transcription request completed filename={} text_len={}",
            filename,
            text.len()
        );
        Ok(AsrResult {
            text,
            is_final: true,
            confidence: 1.0,
        })
    }

    fn transcribe(
        &self,
        audio: BoxStream<'static, Result<Bytes>>,
    ) -> BoxStream<'static, Result<AsrResult>> {
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
                    match transcribe_segment(
                        &client,
                        &endpoint,
                        api_key.as_deref(),
                        &model,
                        &language,
                        &segment,
                    )
                    .await
                    {
                        Ok(text) => {
                            if tx
                                .send(Ok(AsrResult {
                                    text,
                                    is_final: false,
                                    confidence: 1.0,
                                }))
                                .await
                                .is_err()
                            {
                                return;
                            }
                        }
                        Err(e) => {
                            let _ = tx
                                .send(Err(e.context("segment transcription failed")))
                                .await;
                            ok = false;
                        }
                    }
                }
            }

            if ok && !buffer.is_empty() {
                match transcribe_segment(
                    &client,
                    &endpoint,
                    api_key.as_deref(),
                    &model,
                    &language,
                    &buffer,
                )
                .await
                {
                    Ok(text) => {
                        let _ = tx
                            .send(Ok(AsrResult {
                                text,
                                is_final: true,
                                confidence: 1.0,
                            }))
                            .await;
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e.context("final segment failed"))).await;
                    }
                }
            } else if ok {
                let _ = tx
                    .send(Ok(AsrResult {
                        text: String::new(),
                        is_final: true,
                        confidence: 1.0,
                    }))
                    .await;
            }
        });

        tokio_stream::wrappers::ReceiverStream::new(rx).boxed()
    }
}

async fn transcribe_segment(
    client: &reqwest::Client,
    endpoint: &str,
    api_key: Option<&str>,
    model: &str,
    language: &Option<String>,
    pcm_data: &[u8],
) -> Result<String> {
    let wav = crate::pcm_to_wav(pcm_data)?;
    let url = transcription_url(endpoint);

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

    typex_logging::log_target!(
        tracing::Level::DEBUG,
        target: "typex_asr",
        format!(
            "ASR segment transcription started model={} endpoint={} pcm_bytes={}",
            model,
            endpoint,
            pcm_data.len()
        ),
    );
    let text = post_transcription(client, &url, api_key, form).await?;
    typex_logging::log_target!(
        tracing::Level::DEBUG,
        target: "typex_asr",
        "ASR segment transcription completed text_len={}",
        text.len()
    );
    Ok(text)
}

fn transcription_url(endpoint: &str) -> String {
    format!("{}/audio/transcriptions", endpoint.trim_end_matches('/'))
}

async fn post_transcription(
    client: &reqwest::Client,
    url: &str,
    api_key: Option<&str>,
    form: reqwest::multipart::Form,
) -> Result<String> {
    let mut req = client.post(url);
    if let Some(key) = api_key.map(|s| s.trim()).filter(|s| !s.is_empty()) {
        req = req.bearer_auth(key);
    }
    let resp = req
        .multipart(form)
        .send()
        .await
        .map_err(map_request_error)?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(ProviderError::new(
            ProviderService::Asr,
            "openai-compatible",
            kind_from_http_status(status.as_u16()),
            format!(
                "API error {}: {}",
                status,
                typex_logging::text_preview(&body, 200)
            ),
        )
        .with_status(status.as_u16())
        .into());
    }

    let result: TranscriptionResponse = resp.json().await.map_err(|e| {
        ProviderError::new(
            ProviderService::Asr,
            "openai-compatible",
            ProviderErrorKind::InvalidResponse,
            format!("failed to parse API response: {e}"),
        )
    })?;
    Ok(result.text.trim().to_string())
}

fn map_request_error(error: reqwest::Error) -> ProviderError {
    let kind = if error.is_timeout() {
        ProviderErrorKind::Timeout
    } else if error.is_builder() {
        ProviderErrorKind::InvalidEndpoint
    } else {
        ProviderErrorKind::Network
    };

    ProviderError::new(
        ProviderService::Asr,
        "openai-compatible",
        kind,
        format!("API request failed: {error}"),
    )
}

fn guess_mime(filename: &str) -> &'static str {
    match filename
        .rsplit('.')
        .next()
        .map(|s| s.to_lowercase())
        .as_deref()
    {
        Some("mp3") => "audio/mpeg",
        Some("ogg") => "audio/ogg",
        Some("flac") => "audio/flac",
        Some("m4a") => "audio/mp4",
        Some("webm") => "audio/webm",
        _ => "audio/wav",
    }
}
