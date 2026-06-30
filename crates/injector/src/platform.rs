use anyhow::Result;
use tracing::Level;

use crate::Injector;

/// Platform-specific injector. Stubs for now.
pub struct PlatformInjector;

impl Injector for PlatformInjector {
    fn name(&self) -> &str {
        "platform"
    }

    fn inject(&self, text: &str) -> Result<()> {
        #[cfg(target_os = "macos")]
        let platform = "macos";
        #[cfg(target_os = "windows")]
        let platform = "windows";
        #[cfg(target_os = "linux")]
        let platform = "linux";

        // Keep this as a warning while platform injection is a stub: selecting
        // this injector does not actually deliver text to the active app yet.
        // When real platform injection is implemented, remove this warning or
        // lower the success-path log back to DEBUG.
        typex_logging::log_text_target!(
            Level::WARN,
            target: "typex_injector",
            format!("platform injector stub platform={platform}"),
            text,
            false,
        );

        Ok(())
    }
}
