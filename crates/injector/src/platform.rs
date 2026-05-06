use anyhow::Result;

use crate::Injector;

/// Platform-specific injector. Stubs for now.
pub struct PlatformInjector;

impl Injector for PlatformInjector {
    fn name(&self) -> &str {
        "platform"
    }

    fn inject(&self, text: &str) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            // TODO: Accessibility API
            println!("[macos-inject] {}", text);
        }
        #[cfg(target_os = "windows")]
        {
            // TODO: SendInput
            println!("[windows-inject] {}", text);
        }
        #[cfg(target_os = "linux")]
        {
            // TODO: xdotool / ydotool
            println!("[linux-inject] {}", text);
        }
        Ok(())
    }
}
