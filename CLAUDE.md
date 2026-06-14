# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Run

```bash
cargo build                    # build entire workspace
cargo run -p typex-cli         # run CLI (stream mode with microphone)
cargo run -p typex-cli -- --input audio.wav   # file mode: transcribe a WAV file
cargo build -p typex-desktop   # build Tauri desktop app
cargo test                     # run all tests (none yet)
cargo test -p typex-pipeline   # run tests for a single crate
```

## Architecture

Cargo workspace monorepo. All core logic lives in `crates/`, executables in `apps/`.

**Data flow (streaming, never batch):**
```
microphone → capture (cpal) → resample (rubato) → 16kHz mono PCM
    → ASR → text chunk → plugins (sequential) → LLM (optional) → injector → target app
```

**CLI modes:**
- Stream mode (default): microphone capture → pipeline → real-time transcription
- File mode (`--input`): WAV file → ASR → plugins → output text

**Desktop app (`typex-desktop`):** Tauri-based GUI with an overlay window for real-time transcription display. Frontend is pure HTML/CSS/JS with i18n support (EN, zh-CN). Tauri commands bridge the Rust backend (audio capture, pipeline, config) to the frontend UI.

**Key traits and types (the extension points):**

| Trait / Type | Crate | Purpose |
|---|---|---|
| `AsrProvider` | `typex-asr` | `transcribe(audio_stream) → text_stream` |
| `LlmProvider` | `typex-llm` | `optimize(text_stream) → text_stream` |
| `Plugin` | `typex-plugin` | `process(text, ctx) → text` (async, sequential) |
| `Injector` | `typex-injector` | `inject(text)` (system-level input) |
| `MicrophoneCapture` | `typex-audio` | `start() → (Stream, BoxStream<Bytes>)` (16kHz mono PCM) |

**Audio capture (`typex-audio`):** Cross-platform microphone input via cpal. Auto-detects device sample rate, downmixes multi-channel to mono, resamples to 16kHz using rubato if needed. Output is 16-bit little-endian mono PCM `BoxStream`, directly consumable by ASR providers.

**Pipeline (`typex-pipeline`):** Wires everything together via `Pipeline::new(asr).with_llm(...).with_plugin(...).with_injector(...)`. The `run()` method takes an audio stream and returns a `BoxStream<PipelineOutput>`.

**TypeX builder (`typex-core`):** Convenience wrapper that re-exports all crates and provides `TypeX::builder(asr)`.

**Config (`typex-config`):** TOML-based (`config.toml`). `AppConfig` with sub-configs for asr/llm/pipeline/injector/audio/history/shortcut/overlay. Supports serde defaults and `save()` for persisting changes.

## Adding a Provider

1. Create a new module in `crates/asr/src/` (or `crates/llm/src/`)
2. Implement `AsrProvider` (or `LlmProvider`) — the `transcribe`/`optimize` methods must return `BoxStream`
3. Register in CLI based on config string matching

## Adding a Plugin

1. Create a new module in `crates/plugin/src/`
2. Implement `Plugin` trait (name + async process)
3. Add `pub mod` in `crates/plugin/src/lib.rs`
4. Add plugin name to `config.toml` `pipeline.plugins` list
5. Add match arm in CLI main

## Conventions

- All I/O is streaming (`BoxStream`, `futures::StreamExt`). No batch processing as main path.
- Provider traits use `async_trait` and `BoxStream<'static, Result<...>>` signatures.
- Audio output is always 16kHz 16-bit mono PCM (i16 little-endian), regardless of input device format.
- cpal audio callback must not block — only `try_send` to mpsc channel, no allocation-heavy work.
- Mock providers exist under `crates/asr/src/mock.rs` and `crates/llm/src/mock.rs`.
- Config path: `config.toml` in CWD, or `~/.typex/config.toml`.
- License: BSL-1.1 (non-commercial use permitted, commercial requires license).
