# TypeX

A system-level AI voice input tool. Real-time streaming ASR → text optimization → system input injection.

## Quick Start

```bash
# Build entire workspace
cargo build

# CLI — stream mode (microphone capture + real-time transcription)
cargo run -p typex-cli

# CLI — file mode (transcribe a WAV file)
cargo run -p typex-cli -- --input audio.wav

# Desktop — build the Tauri GUI app
cargo build -p typex-desktop

# Customize config
cp config.example.toml config.toml
# Edit config.toml with your API keys
```

## Architecture

```
microphone → capture → resample (→ 16kHz mono PCM) → ASR → text → plugins → LLM (optional) → injector → target app
```

## Apps

| App | Description |
|-----|-------------|
| `typex-cli` | Terminal CLI — stream mode (mic) or file mode (--input) |
| `typex-desktop` | Tauri desktop app with overlay UI and i18n (EN / zh-CN) |

## Crates

| Crate | Description |
|-------|-------------|
| `typex-core` | Top-level re-exports and builder |
| `typex-audio` | Microphone capture with cpal + rubato resampling |
| `typex-asr` | ASR provider trait + implementations |
| `typex-llm` | LLM provider trait + implementations |
| `typex-pipeline` | Streaming pipeline orchestration |
| `typex-plugin` | Plugin trait + built-in plugins |
| `typex-injector` | System input injection |
| `typex-config` | TOML configuration (ASR, LLM, pipeline, injector, audio, history, shortcuts, overlay) |

## Configuration

`AppConfig` supports these sections in `config.toml`:

- **asr** — provider selection and model
- **llm** — provider, model, prompt
- **pipeline** — plugins list, streaming mode
- **injector** — method (clipboard / keyboard), settle delay
- **audio** — device override
- **history** — transcription log limit, recording session limit
- **shortcut** — record toggle hotkey (e.g. `Ctrl+Alt+Space`)
- **overlay** — desktop overlay window settings

## Adding a Provider

1. Implement the trait (`AsrProvider` or `LlmProvider`) in its crate
2. Add a module under `crates/asr/src/` or `crates/llm/src/`
3. Register in the CLI/main based on config

## Adding a Plugin

1. Implement `Plugin` trait in `crates/plugin/src/`
2. Add module to `crates/plugin/src/lib.rs`
3. Add name to `config.toml` plugins list

## License

BSL-1.1 — non-commercial use permitted; commercial use requires a license.
