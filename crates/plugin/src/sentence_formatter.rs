use anyhow::Result;
use async_trait::async_trait;

use crate::{Plugin, PluginContext};

/// Adds punctuation between sentences (auto-break).
pub struct SentenceFormatter;

#[async_trait]
impl Plugin for SentenceFormatter {
    fn name(&self) -> &str {
        "sentence_formatter"
    }

    async fn process(&self, text: &str, ctx: &PluginContext) -> Result<String> {
        if !ctx.is_final {
            return Ok(text.to_string());
        }
        // Naive: ensure trailing period for final chunks
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Ok(text.to_string());
        }
        if trimmed.ends_with('。') || trimmed.ends_with('！') || trimmed.ends_with('？') {
            Ok(text.to_string())
        } else {
            Ok(format!("{}。", trimmed))
        }
    }
}
