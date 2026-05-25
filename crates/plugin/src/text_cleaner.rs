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
        let words: Vec<&str> = text.split_whitespace().collect();
        let mut result = String::new();
        for (i, word) in words.iter().enumerate() {
            if i > 0 {
                let prev_ends_cjk = words[i - 1].ends_with(is_cjk);
                let curr_starts_cjk = word.starts_with(is_cjk);
                if !prev_ends_cjk || !curr_starts_cjk {
                    result.push(' ');
                }
            }
            result.push_str(word);
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
        | '\u{AC00}'..='\u{D7AF}'
        | '\u{FF00}'..='\u{FFEF}'
    )
}
