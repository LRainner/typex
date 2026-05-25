use anyhow::Result;

use crate::Injector;

/// Fallback injector that copies text to clipboard and simulates Cmd+V / Ctrl+V.
pub struct ClipboardInjector;

impl Injector for ClipboardInjector {
    fn name(&self) -> &str {
        "clipboard"
    }

    fn inject(&self, text: &str) -> Result<()> {
        // TODO: use arboard or Cocoa NSPasteboard
        tracing::debug!("clipboard-inject: {}", text);
        Ok(())
    }
}
