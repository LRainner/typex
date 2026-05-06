use anyhow::Result;

/// System-level text input injector.
pub trait Injector: Send + Sync {
    fn name(&self) -> &str;

    /// Type `text` into the currently focused application.
    fn inject(&self, text: &str) -> Result<()>;
}

pub mod clipboard;
pub mod platform;
