use anyhow::Result;
use futures::StreamExt;
use std::sync::Arc;

use typex_asr::mock::MockAsrProvider;
use typex_asr::openai_compat::OpenAiCompatibleAsrProvider;
use typex_asr::AsrProvider;
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

    let asr = create_asr_provider(&config)?;

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

fn create_asr_provider(config: &AppConfig) -> Result<Arc<dyn AsrProvider>> {
    match config.asr.provider.as_str() {
        "mock" => Ok(Arc::new(MockAsrProvider::new())),
        "openai-compatible" => {
            let endpoint = config.asr.endpoint.as_deref().unwrap_or("https://api.openai.com/v1");
            let api_key = config.asr.api_key.clone()
                .or_else(|| std::env::var("OPENAI_API_KEY").ok())
                .ok_or_else(|| anyhow::anyhow!("ASR api_key not set (config or OPENAI_API_KEY env)"))?;
            let model = config.asr.model.as_deref().unwrap_or("whisper-1");

            let mut provider = OpenAiCompatibleAsrProvider::new(
                endpoint.to_string(),
                api_key,
                model.to_string(),
            );

            if let Some(lang) = &config.asr.language {
                provider = provider.with_language(lang.clone());
            }

            Ok(Arc::new(provider))
        }
        other => Err(anyhow::anyhow!("unknown ASR provider: {}", other)),
    }
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
