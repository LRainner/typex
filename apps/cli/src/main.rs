use anyhow::Result;
use futures::StreamExt;
use std::sync::Arc;

use typex_asr::AsrProvider;
use typex_asr::mock::MockAsrProvider;
use typex_asr::openai_compat::OpenAiCompatibleAsrProvider;
use typex_config::AppConfig;
use typex_injector::clipboard::ClipboardInjector;
use typex_plugin::{
    filler_remover::FillerRemover, sentence_formatter::SentenceFormatter, text_cleaner::TextCleaner,
};

use typex_core::TypeX;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("typex=debug")
        .init();

    let config = load_config()?;
    let input_file = parse_input_arg()?;

    if let Some(path) = &input_file {
        // File mode: send audio directly to API, then apply plugins
        validate_file_provider(&config)?;
        let provider = create_openai_provider(&config)?;
        let file_data = tokio::fs::read(path).await?;
        let filename = std::path::Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("audio.wav");

        tracing::info!("transcribing {} ({} bytes)", filename, file_data.len());
        let result = provider.transcribe_file(file_data, filename).await?;

        let text = apply_plugins(&result.text, &config).await;

        if text.is_empty() {
            println!("(no speech detected)");
        } else {
            println!("{}", text);
        }
    } else {
        // Stream mode: pipeline with microphone
        let asr: Arc<dyn AsrProvider> = match config.asr.provider.as_str() {
            "mock" => Arc::new(MockAsrProvider::new()),
            "openai-compatible" | "" => create_openai_provider(&config)?,
            other => anyhow::bail!("unknown ASR provider: {}", other),
        };

        let mut builder = TypeX::builder(asr);
        for name in &config.pipeline.plugins {
            if let Some(plugin) = create_plugin(name) {
                builder = builder.plugin(plugin);
            } else {
                tracing::warn!("unknown plugin: {}", name);
            }
        }
        builder = builder.injector(Arc::new(ClipboardInjector));
        let typex = builder.build();

        let capture = typex_audio::MicrophoneCapture::new(config.audio.device.clone());
        let (_stream_handle, audio) = capture.start()?;
        let mut stream = typex.pipeline().run(audio);

        let ctrl_c = tokio::signal::ctrl_c();
        tokio::pin!(ctrl_c);

        loop {
            tokio::select! {
                _ = &mut ctrl_c => {
                    tracing::info!("stopping capture...");
                    break;
                }
                result = stream.next() => {
                    match result {
                        Some(Ok(output)) => {
                            let tag = if output.is_final { "FINAL" } else { "partial" };
                            println!("[{}] {}", tag, output.text);
                        }
                        Some(Err(e)) => {
                            tracing::error!("pipeline error: {}", e);
                            break;
                        }
                        None => break,
                    }
                }
            }
        }
    }

    Ok(())
}

async fn apply_plugins(text: &str, config: &AppConfig) -> String {
    let ctx = typex_plugin::PluginContext { is_final: true };
    let mut result = text.to_string();
    for name in &config.pipeline.plugins {
        if let Some(plugin) = create_plugin(name) {
            match plugin.process(&result, &ctx).await {
                Ok(processed) => result = processed,
                Err(e) => tracing::warn!("plugin {} failed: {}", name, e),
            }
        }
    }
    result
}

fn create_plugin(name: &str) -> Option<Arc<dyn typex_plugin::Plugin>> {
    match name {
        "filler_remover" => Some(Arc::new(FillerRemover)),
        "sentence_formatter" => Some(Arc::new(SentenceFormatter)),
        "text_cleaner" => Some(Arc::new(TextCleaner)),
        _ => None,
    }
}

fn validate_file_provider(config: &AppConfig) -> Result<()> {
    match config.asr.provider.as_str() {
        "openai-compatible" | "" => Ok(()),
        "mock" => anyhow::bail!("mock provider does not support file transcription"),
        other => anyhow::bail!("unknown ASR provider: {}", other),
    }
}

fn create_openai_provider(config: &AppConfig) -> Result<Arc<OpenAiCompatibleAsrProvider>> {
    let endpoint = config
        .asr
        .endpoint
        .as_deref()
        .unwrap_or("https://api.openai.com/v1");
    let api_key = config
        .asr
        .api_key
        .clone()
        .or_else(|| std::env::var("OPENAI_API_KEY").ok())
        .ok_or_else(|| anyhow::anyhow!("ASR api_key not set (config or OPENAI_API_KEY env)"))?;
    let model = config.asr.model.as_deref().unwrap_or("whisper-1");

    let mut provider =
        OpenAiCompatibleAsrProvider::new(endpoint.to_string(), api_key, model.to_string());

    if let Some(lang) = &config.asr.language {
        provider = provider.with_language(lang.clone());
    }

    Ok(Arc::new(provider))
}

fn parse_input_arg() -> Result<Option<String>> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--input" {
            let path = args
                .next()
                .ok_or_else(|| anyhow::anyhow!("--input requires a file path"))?;
            return Ok(Some(path));
        }
    }
    Ok(None)
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
