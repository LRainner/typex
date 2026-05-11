# TypeX

A system-level AI voice input tool. Real-time streaming ASR → text optimization → system input injection.

## Quick Start

```bash
# Build
cargo build

# Run CLI demo
cargo run -p typex-cli

# Customize config
cp config.example.toml config.toml
# Edit config.toml with your API keys
```

## Architecture

```
audio → ASR → text chunk → plugins → LLM (optional) → injector → target app
```

## Crates

| Crate | Description |
|-------|-------------|
| `typex-core` | Top-level re-exports and builder |
| `typex-asr` | ASR provider trait + implementations |
| `typex-llm` | LLM provider trait + implementations |
| `typex-pipeline` | Streaming pipeline orchestration |
| `typex-plugin` | Plugin trait + built-in plugins |
| `typex-injector` | System input injection |
| `typex-config` | TOML configuration |

## Adding a Provider

1. Implement the trait (`AsrProvider` or `LLMProvider`) in its crate
2. Add a module under `crates/asr/src/` or `crates/llm/src/`
3. Register in the CLI/main based on config

## Adding a Plugin

1. Implement `Plugin` trait in `crates/plugin/src/`
2. Add module to `crates/plugin/src/lib.rs`
3. Add name to `config.toml` plugins list
