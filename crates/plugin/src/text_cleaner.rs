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
        let mut words = text.split_whitespace().peekable();
        let mut result = String::with_capacity(text.len());

        while let Some(word) = words.next() {
            result.push_str(word);
            if let Some(next) = words.peek()
                && (!word.ends_with(is_cjk) || !next.starts_with(is_cjk))
            {
                result.push(' ');
            }
        }

        Ok(result)
    }
}

fn is_cjk(ch: char) -> bool {
    matches!(ch,
        '\u{4E00}'..='\u{9FFF}'
        | '\u{3400}'..='\u{4DBF}'
        | '\u{F900}'..='\u{FAFF}'
        | '\u{3000}'..='\u{303F}'
        | '\u{3040}'..='\u{309F}'
        | '\u{30A0}'..='\u{30FF}'
        | '\u{FF00}'..='\u{FFEF}'
    )
}
