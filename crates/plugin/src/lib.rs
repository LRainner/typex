use anyhow::Result;
use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct PluginContext {
    pub is_final: bool,
}

/// A plugin transforms a text chunk. Simple and synchronous for now —
/// the pipeline calls each plugin in sequence.
#[async_trait]
pub trait Plugin: Send + Sync {
    fn name(&self) -> &str;

    /// Transform text. Return the modified string.
    async fn process(&self, text: &str, ctx: &PluginContext) -> Result<String>;
}

pub mod filler_remover;
pub mod sentence_formatter;
pub mod text_cleaner;
