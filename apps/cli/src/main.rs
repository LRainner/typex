use anyhow::Result;
use futures::StreamExt;
use std::sync::Arc;

use typex_asr::mock::MockAsrProvider;
use typex_config::AppConfig;
use typex_injector::clipboard::ClipboardInjector;
use typex_plugin::{filler_remover::FillerRemover, sentence_formatter::SentenceFormatter, text_cleaner::TextCleaner};

use typex_core::TypeX;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("typex=debug")
        .init();

    let config = load_config()?;

    let asr = Arc::new(MockAsrProvider::new());

    let mut builder = TypeX::builder(asr);

    for name in &config.pipeline.plugins {
        builder = match name.as_str() {
            "filler_remover" => builder.plugin(Arc::new(FillerRemover)),
            "sentence_formatter" => builder.plugin(Arc::new(SentenceFormatter)),
            "text_cleaner" => builder.plugin(Arc::new(TextCleaner)),
            other => {
                tracing::warn!("unknown plugin: {}", other);
                builder
            }
        };
    }

    builder = builder.injector(Arc::new(ClipboardInjector));

    let typex = builder.build();

    // Empty audio stream — mock ASR ignores it and produces its own chunks
    let audio = futures::stream::empty::<Result<bytes::Bytes>>().boxed();

    println!("=== TypeX Pipeline Demo ===\n");

    let mut stream = typex.pipeline().run(audio);

    while let Some(result) = stream.next().await {
        match result {
            Ok(output) => {
                if output.is_final {
                    println!("[FINAL] {}", output.text);
                } else {
                    println!("[partial] {}", output.text);
                }
            }
            Err(e) => {
                tracing::error!("pipeline error: {}", e);
                break;
            }
        }
    }

    println!("\n=== Pipeline Complete ===");
    Ok(())
}

fn load_config() -> Result<AppConfig> {
    let config_path = std::path::Path::new("config.toml");
    if config_path.exists() {
        let config = AppConfig::load(config_path)?;
        tracing::info!("loaded config from {}", config_path.display());
        Ok(config)
    } else {
        tracing::info!("using default config");
        Ok(AppConfig::default())
    }
}
