use anyhow::Result;
use async_trait::async_trait;

use crate::{Plugin, PluginContext};

/// Strips extra whitespace and normalizes text.
pub struct TextCleaner;

#[async_trait]
impl Plugin for TextCleaner {
    fn name(&self) -> &str {
        "text_cleaner"
    }

    async fn process(&self, text: &str, _ctx: &PluginContext) -> Result<String> {
        let cleaned = text.split_whitespace().collect::<Vec<_>>().join("");
        Ok(cleaned)
    }
}
