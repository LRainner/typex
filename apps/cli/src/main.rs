use anyhow::Result;
use tokio::io::AsyncBufReadExt;

use tracing::Level;
use typex_config::{AppConfig, LogLevel};
use typex_core::{TypeXBuildOptions, build_typex_from_config};

#[tokio::main]
async fn main() -> Result<()> {
    let config_path = std::path::Path::new("config.toml");
    let config_exists = config_path.exists();
    let config = load_config(config_path)?;
    init_tracing(config.logging.level);
    if config_exists {
        typex_logging::log_target!(
            Level::INFO,
            target: "typex_cli",
            "loaded config from {}",
            config_path.display()
        );
    } else {
        typex_logging::log_target!(Level::INFO, target: "typex_cli", "using default config");
    }

    let input_file = parse_input_arg()?;
    let options = if input_file.is_some() {
        TypeXBuildOptions::file()
    } else {
        TypeXBuildOptions::session()
    };
    let typex = build_typex_from_config(&config, options)?;

    if let Some(path) = &input_file {
        // File mode: transcribe audio file
        let file_data = tokio::fs::read(path).await?;
        let filename = std::path::Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("audio.wav");

        typex_logging::log_target!(
            Level::INFO,
            target: "typex_cli",
            "transcribing {} ({} bytes)",
            filename,
            file_data.len()
        );
        let result = typex.run_file(file_data, filename).await?;
        if result.text.is_empty() {
            println!("(no speech detected)");
        } else {
            println!("{}", result.text);
        }
    } else {
        // Session mode: press Enter to start/stop recording
        let capture = typex_audio::MicrophoneCapture::new(config.audio.device.clone());

        println!("TypeX session mode. Press Enter to start/stop recording.");

        let stdin = tokio::io::BufReader::new(tokio::io::stdin());
        let mut lines = stdin.lines();

        loop {
            println!("\nPress Enter to start recording...");
            lines
                .next_line()
                .await?
                .ok_or_else(|| anyhow::anyhow!("stdin closed"))?;

            let recorder = capture.record_session()?;
            println!("Recording... Press Enter to stop.");

            lines
                .next_line()
                .await?
                .ok_or_else(|| anyhow::anyhow!("stdin closed"))?;

            let pcm = recorder.stop().await?;
            typex_logging::log_target!(
                Level::DEBUG,
                target: "typex_cli",
                "captured {} bytes of PCM audio",
                pcm.len()
            );

            if pcm.is_empty() {
                println!("(no audio captured)");
                continue;
            }

            let result = typex.run_session(pcm).await?;
            if result.text.is_empty() {
                println!("(no speech detected)");
            } else {
                println!("{}", result.text);
            }
        }
    }

    Ok(())
}

fn init_tracing(level: LogLevel) {
    let env_filter = tracing_subscriber::EnvFilter::new(typex_logging::build_filter(
        level.as_str(),
        &["typex_cli"],
    ));
    tracing_subscriber::fmt().with_env_filter(env_filter).init();
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

fn load_config(config_path: &std::path::Path) -> Result<AppConfig> {
    if config_path.exists() {
        AppConfig::load(config_path)
    } else {
        Ok(AppConfig::default())
    }
}
