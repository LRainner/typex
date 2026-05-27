use anyhow::Result;

use crate::Injector;

/// Injects text by writing to the clipboard and simulating Ctrl+V / Cmd+V.
pub struct ClipboardInjector;

impl Injector for ClipboardInjector {
    fn name(&self) -> &str {
        "clipboard"
    }

    fn inject(&self, text: &str) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        let mut clipboard = arboard::Clipboard::new()?;
        clipboard.set_text(text)?;
        std::thread::sleep(std::time::Duration::from_millis(100));
        simulate_paste()?;
        Ok(())
    }
}

fn simulate_paste() -> Result<()> {
    use enigo::{Direction, Key, Keyboard, Settings};
    let mut enigo = enigo::Enigo::new(&Settings::default())?;

    let modifier = if cfg!(target_os = "macos") {
        Key::Meta
    } else {
        Key::Control
    };

    enigo.key(modifier, Direction::Press)?;
    let click_res = enigo.key(Key::Unicode('v'), Direction::Click);
    let release_res = enigo.key(modifier, Direction::Release);
    click_res.and(release_res)?;
    Ok(())
}
