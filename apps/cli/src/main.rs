use anyhow::Result;
use futures::stream::{BoxStream, StreamExt};
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
    let input_file = parse_input_arg();

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

    let audio = create_audio_stream(&config, input_file.as_deref())?;

    println!("=== TypeX Pipeline ===\n");

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

fn create_audio_stream(config: &AppConfig, input_file: Option<&str>) -> Result<BoxStream<'static, Result<bytes::Bytes>>> {
    match config.asr.provider.as_str() {
        "mock" => {
            // Mock provider generates its own data, empty stream is fine
            Ok(futures::stream::empty::<Result<bytes::Bytes>>().boxed())
        }
        "openai-compatible" => {
            let path = input_file.ok_or_else(|| anyhow::anyhow!("--input <file> required for openai-compatible provider"))?;
            let data = read_wav_pcm(path)?;
            tracing::info!("loaded {} bytes of PCM from {}", data.len(), path);
            let stream = pcm_chunk_stream(data, 8192);
            Ok(stream.boxed())
        }
        other => Err(anyhow::anyhow!("unknown ASR provider: {}", other)),
    }
}

/// Read a WAV file and return raw PCM data (skipping the 44-byte header).
fn read_wav_pcm(path: &str) -> Result<Vec<u8>> {
    let file = std::fs::read(path)?;
    if file.len() < 44 {
        anyhow::bail!("WAV file too small: {} bytes", file.len());
    }
    if &file[0..4] != b"RIFF" || &file[8..12] != b"WAVE" {
        anyhow::bail!("not a valid WAV file: {}", path);
    }
    // Skip the 44-byte WAV header — assumes 16kHz 16-bit mono
    Ok(file[44..].to_vec())
}

/// Convert a PCM byte vector into a chunked stream of Bytes.
fn pcm_chunk_stream(data: Vec<u8>, chunk_size: usize) -> BoxStream<'static, Result<bytes::Bytes>> {
    let chunks: Vec<Result<bytes::Bytes>> = data
        .chunks(chunk_size)
        .map(|c| Ok(bytes::Bytes::copy_from_slice(c)))
        .collect();
    futures::stream::iter(chunks).boxed()
}

/// Parse --input <file> from command line args.
fn parse_input_arg() -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    for i in 0..args.len() {
        if args[i] == "--input" && i + 1 < args.len() {
            return Some(args[i + 1].clone());
        }
    }
    None
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
