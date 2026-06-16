use crate::{LlmProvider, LlmResult};
use anyhow::Result;
use async_trait::async_trait;
use futures::stream::{BoxStream, StreamExt};
use serde::Deserialize;

/// Default system prompt for text optimization.
const DEFAULT_SYSTEM_PROMPT: &str = "\
You are a text optimization assistant. Your task is to improve the clarity, \
grammar, and fluency of the input text. \
Rules:\n\
- Fix typos, grammar errors, and awkward phrasing.\n\
- Add proper punctuation and capitalization if missing.\n\
- Preserve the original meaning and tone.\n\
- Output ONLY the optimized text. No explanations, no markdown, no quotes.\n\
- If the input is already correct, output it unchanged.";

/// OpenAI-compatible LLM provider for text optimization.
///
/// Uses the Chat Completions API with streaming (SSE). Incoming text
/// chunks are collected, combined, and sent as a single user message.
/// The streaming response is yielded chunk-by-chunk as `LlmResult`.
pub struct OpenAiCompatibleLlmProvider {
    endpoint: String,
    api_key: Option<String>,
    model: String,
    system_prompt: String,
    client: reqwest::Client,
}

impl OpenAiCompatibleLlmProvider {
    pub fn new(endpoint: String, api_key: Option<String>, model: String) -> Self {
        Self {
            endpoint,
            api_key,
            model,
            system_prompt: DEFAULT_SYSTEM_PROMPT.to_string(),
            client: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }

    pub fn with_system_prompt(mut self, prompt: String) -> Self {
        if !prompt.trim().is_empty() {
            self.system_prompt = prompt;
        }
        self
    }
}

#[derive(Deserialize)]
struct ChatCompletionChunk {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    delta: Delta,
}

#[derive(Deserialize, Default)]
struct Delta {
    #[serde(default)]
    content: Option<String>,
}

#[async_trait]
impl LlmProvider for OpenAiCompatibleLlmProvider {
    fn name(&self) -> &str {
        "openai-compatible-llm"
    }

    fn optimize(
        &self,
        text: BoxStream<'static, Result<String>>,
    ) -> BoxStream<'static, Result<LlmResult>> {
        let endpoint = chat_url(&self.endpoint);
        let api_key = self.api_key.clone();
        let model = self.model.clone();
        let system_prompt = self.system_prompt.clone();
        let client = self.client.clone();

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<LlmResult>>(16);

        tokio::spawn(async move {
            // ── 1. Collect input text ──
            let mut text = Box::pin(text);
            let mut input = String::new();
            while let Some(chunk) = text.next().await {
                match chunk {
                    Ok(t) => {
                        if !input.is_empty() {
                            input.push(' ');
                        }
                        input.push_str(t.trim());
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e.context("LLM input stream error"))).await;
                        return;
                    }
                }
            }

            if input.trim().is_empty() {
                let _ = tx
                    .send(Ok(LlmResult {
                        text: String::new(),
                        is_final: true,
                    }))
                    .await;
                return;
            }

            // ── 2. Build request body ──
            let body = serde_json::json!({
                "model": model,
                "messages": [
                    {"role": "system", "content": system_prompt},
                    {"role": "user", "content": input}
                ],
                "stream": true,
                "temperature": 0.3,
                "max_tokens": 2048
            });

            // ── 3. Send streaming request ──
            let mut req = client
                .post(&endpoint)
                .header("Content-Type", "application/json");

            if let Some(ref key) = api_key {
                let key = key.trim();
                if !key.is_empty() {
                    req = req.bearer_auth(key);
                }
            }

            let resp = match req.json(&body).send().await {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx
                        .send(Err(anyhow::anyhow!("LLM API request failed: {}", e)))
                        .await;
                    return;
                }
            };

            let status = resp.status();
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                let _ = tx
                    .send(Err(anyhow::anyhow!("LLM API error {}: {}", status, body)))
                    .await;
                return;
            }

            // ── 4. Parse SSE stream ──
            // Use Vec<u8> buffer to safely accumulate raw bytes across TCP chunks.
            // This avoids corrupting multi-byte UTF-8 characters that may be split
            // across chunk boundaries (String::from_utf8_lossy would replace a split
            // codepoint with U+FFFD and the character would be lost forever).
            let mut stream = resp.bytes_stream();
            let mut buf: Vec<u8> = Vec::new();
            let mut final_text = String::new();

            'stream: while let Some(result) = stream.next().await {
                let bytes = match result {
                    Ok(b) => b,
                    Err(e) => {
                        let _ = tx.send(Err(anyhow::anyhow!("SSE read error: {}", e))).await;
                        return;
                    }
                };

                buf.extend_from_slice(&bytes);

                // Process complete SSE lines (delimited by \n)
                while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                    let line_bytes: Vec<u8> = buf.drain(..pos).collect();
                    buf.remove(0); // discard the \n
                    // Strip trailing \r if present (SSE lines may end with \r\n)
                    let line_bytes = line_bytes.strip_suffix(b"\r").unwrap_or(&line_bytes);
                    let line = String::from_utf8_lossy(line_bytes);
                    let line = line.trim();

                    if line.is_empty() || line.starts_with(':') {
                        continue;
                    }

                    if let Some(data) = line.strip_prefix("data:") {
                        let data = data.trim();
                        if data == "[DONE]" {
                            break 'stream;
                        }
                        match serde_json::from_str::<ChatCompletionChunk>(data) {
                            Ok(chunk) => {
                                if let Some(content) = chunk
                                    .choices
                                    .first()
                                    .and_then(|c| c.delta.content.as_deref())
                                {
                                    final_text.push_str(content);
                                    if tx
                                        .send(Ok(LlmResult {
                                            text: content.to_string(),
                                            is_final: false,
                                        }))
                                        .await
                                        .is_err()
                                    {
                                        return; // receiver dropped
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "failed to parse SSE chunk: {} (data: {})",
                                    e,
                                    &data[..data.len().min(200)]
                                );
                            }
                        }
                    }
                }
            }

            // ── 5. Final marker ──
            let _ = tx
                .send(Ok(LlmResult {
                    text: String::new(),
                    is_final: true,
                }))
                .await;
        });

        tokio_stream::wrappers::ReceiverStream::new(rx).boxed()
    }
}

fn chat_url(endpoint: &str) -> String {
    format!("{}/chat/completions", endpoint.trim_end_matches('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chat_url_strips_trailing_slash() {
        assert_eq!(
            chat_url("https://api.openai.com/v1/"),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn test_chat_url_no_trailing_slash() {
        assert_eq!(
            chat_url("https://api.openai.com/v1"),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn test_default_system_prompt_is_non_empty() {
        assert!(!DEFAULT_SYSTEM_PROMPT.trim().is_empty());
    }
}
