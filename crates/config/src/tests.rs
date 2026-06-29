use super::*;

/// Simulate the exact data flow: frontend JSON → Rust AppConfig → TOML → verify.
#[test]
fn test_llm_config_roundtrip_from_legacy_json() {
    // This is what the previous frontend shape sent via invoke('save_config', { config: ... }).
    let json = serde_json::json!({
        "asr": {
            "provider": "openai-compatible",
            "endpoint": "http://127.0.0.1:8080",
            "model": "qwen3-asr-0.6b",
            "api_key": null,
            "language": null
        },
        "llm": {
            "enabled": true,
            "provider": "openai-compatible",
            "endpoint": "https://api.openai.com/v1",
            "model": "gpt-4o-mini",
            "api_key": "sk-test-key-12345",
            "prompt": "You are a helpful assistant."
        },
        "pipeline": {
            "performance": "low",
            "plugins": ["filler_remover", "sentence_formatter", "text_cleaner"]
        },
        "injector": {
            "method": "clipboard"
        },
        "audio": {
            "device": null
        },
        "history": {
            "log_limit": 0,
            "recording_limit": 50
        },
        "shortcut": {
            "record": "Ctrl+Numpad1"
        },
        "overlay": {
            "x": 1215.0,
            "y": 1127.0
        },
        "ui": {
            "language": "auto"
        }
    });

    let mut config: AppConfig = serde_json::from_value(json).expect("JSON → AppConfig should work");
    config.normalize_connections_mut();

    let asr = config.asr.active_connection_config().unwrap();
    assert_eq!(asr.provider, "openai-compatible");
    assert_eq!(asr.endpoint.as_deref(), Some("http://127.0.0.1:8080"));
    assert_eq!(asr.model.as_deref(), Some("qwen3-asr-0.6b"));

    let llm = config.llm.active_connection_config().unwrap();
    assert_eq!(config.llm.enabled, true);
    assert_eq!(llm.provider, "openai-compatible");
    assert_eq!(llm.endpoint.as_deref(), Some("https://api.openai.com/v1"));
    assert_eq!(llm.model.as_deref(), Some("gpt-4o-mini"));
    assert_eq!(llm.api_key.as_deref(), Some("sk-test-key-12345"));
    assert_eq!(
        config.llm.prompt.as_deref(),
        Some("You are a helpful assistant.")
    );

    assert_eq!(config.logging.level, LogLevel::Info);
    assert_eq!(config.logging.record_text, false);

    let toml_str = toml::to_string_pretty(&config).expect("AppConfig → TOML should work");
    assert!(toml_str.contains("connections"));
    assert!(toml_str.contains("active_connection"));
    assert!(toml_str.contains("sk-test-key-12345"));

    let mut config2: AppConfig = toml::from_str(&toml_str).expect("TOML → AppConfig should work");
    config2.normalize_connections_mut();
    assert_eq!(
        config2
            .llm
            .active_connection_config()
            .unwrap()
            .endpoint
            .as_deref(),
        Some("https://api.openai.com/v1")
    );
}

#[test]
fn test_legacy_toml_normalizes_to_connections() {
    let toml_str = r#"
[asr]
provider = "openai-compatible"
endpoint = "http://127.0.0.1:8080/v1"
model = "whisper-local"
language = "zh"

[llm]
enabled = true
provider = "openai-compatible"
endpoint = "https://openrouter.ai/api/v1"
model = "anthropic/claude-sonnet"
api_key = "sk-test"
prompt = "Polish this text."
"#;

    let mut config: AppConfig = toml::from_str(toml_str).unwrap();
    config.normalize_connections_mut();

    assert_eq!(config.asr.active_connection, "default");
    assert_eq!(config.asr.connections.len(), 1);
    let asr = config.asr.active_connection_config().unwrap();
    assert_eq!(asr.provider, "openai-compatible");
    assert_eq!(asr.endpoint.as_deref(), Some("http://127.0.0.1:8080/v1"));
    assert_eq!(asr.model.as_deref(), Some("whisper-local"));
    assert_eq!(asr.language.as_deref(), Some("zh"));

    assert_eq!(config.llm.active_connection, "default");
    assert_eq!(config.llm.connections.len(), 1);
    let llm = config.llm.active_connection_config().unwrap();
    assert_eq!(llm.provider, "openai-compatible");
    assert_eq!(
        llm.endpoint.as_deref(),
        Some("https://openrouter.ai/api/v1")
    );
    assert_eq!(llm.model.as_deref(), Some("anthropic/claude-sonnet"));
    assert_eq!(llm.api_key.as_deref(), Some("sk-test"));
    assert_eq!(config.llm.prompt.as_deref(), Some("Polish this text."));
}

#[test]
fn test_new_connection_toml_uses_active_connection() {
    let toml_str = r#"
[asr]
provider = "mock"
active_connection = "local-asr"

[[asr.connections]]
id = "openai-asr"
name = "OpenAI ASR"
provider = "openai-compatible"
endpoint = "https://api.openai.com/v1"
model = "whisper-1"

[[asr.connections]]
id = "local-asr"
name = "Local ASR"
provider = "openai-compatible"
endpoint = "http://127.0.0.1:8080/v1"
model = "local-whisper"
language = "en"

[llm]
enabled = true
provider = "mock"
active_connection = "openrouter"
prompt = "Polish this text."

[[llm.connections]]
id = "local-llm"
name = "Local LLM"
provider = "openai-compatible"
endpoint = "http://127.0.0.1:11434/v1"
model = "qwen"

[[llm.connections]]
id = "openrouter"
name = "OpenRouter"
provider = "openai-compatible"
endpoint = "https://openrouter.ai/api/v1"
model = "anthropic/claude-sonnet"
"#;

    let mut config: AppConfig = toml::from_str(toml_str).unwrap();
    config.normalize_connections_mut();

    let asr = config.asr.active_connection_config().unwrap();
    assert_eq!(asr.id, "local-asr");
    assert_eq!(asr.endpoint.as_deref(), Some("http://127.0.0.1:8080/v1"));
    assert_eq!(config.asr.provider, "openai-compatible");
    assert_eq!(config.asr.model.as_deref(), Some("local-whisper"));

    let llm = config.llm.active_connection_config().unwrap();
    assert_eq!(llm.id, "openrouter");
    assert_eq!(
        llm.endpoint.as_deref(),
        Some("https://openrouter.ai/api/v1")
    );
    assert_eq!(config.llm.provider, "openai-compatible");
    assert_eq!(config.llm.model.as_deref(), Some("anthropic/claude-sonnet"));
    assert_eq!(config.llm.prompt.as_deref(), Some("Polish this text."));
}

#[test]
fn test_missing_active_connection_falls_back_to_first() {
    let toml_str = r#"
[asr]
active_connection = "missing"

[[asr.connections]]
id = "first-asr"
name = "First ASR"
provider = "mock"

[llm]
active_connection = "missing"

[[llm.connections]]
id = "first-llm"
name = "First LLM"
provider = "mock"
"#;

    let mut config: AppConfig = toml::from_str(toml_str).unwrap();
    config.normalize_connections_mut();

    assert_eq!(config.asr.active_connection, "first-asr");
    assert_eq!(config.llm.active_connection, "first-llm");
}

/// Verify that Option::None fields are omitted from TOML (expected TOML behavior).
#[test]
fn test_option_none_omitted_from_toml() {
    let json = serde_json::json!({
        "asr": {
            "provider": "mock",
            "endpoint": null,
            "model": null,
            "api_key": null,
            "language": null
        },
        "llm": {
            "enabled": false,
            "provider": "mock",
            "endpoint": null,
            "model": null,
            "api_key": null,
            "prompt": ""
        },
        "pipeline": { "performance": "low", "plugins": [] },
        "injector": { "method": "clipboard" },
        "audio": { "device": null },
        "history": { "log_limit": 0, "recording_limit": 50 },
        "shortcut": { "record": "Ctrl+Alt+Space" },
        "overlay": { "x": null, "y": null },
        "ui": { "language": "auto" }
    });

    let mut config: AppConfig = serde_json::from_value(json).unwrap();
    config.normalize_connections_mut();
    let toml_str = toml::to_string_pretty(&config).unwrap();

    assert!(
        !toml_str.contains("endpoint"),
        "None endpoint should be omitted from TOML"
    );
    assert!(
        !toml_str.contains("api_key"),
        "None api_key should be omitted from TOML"
    );
    assert!(
        !toml_str.contains("model = "),
        "None model should be omitted from TOML"
    );
    assert!(
        toml_str.contains("prompt"),
        "Empty prompt string should appear in TOML"
    );
    assert!(
        toml_str.contains("enabled"),
        "enabled bool should appear in TOML"
    );
}

#[test]
fn test_logging_level_deserializes_from_config() {
    let toml_str = r#"
[logging]
level = "debug"
"#;

    let config: AppConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(config.logging.level, LogLevel::Debug);
    assert_eq!(config.logging.record_text, false);
}

#[test]
fn test_logging_record_text_deserializes_from_config() {
    let toml_str = r#"
[logging]
level = "debug"
record_text = true
"#;

    let config: AppConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(config.logging.level, LogLevel::Debug);
    assert_eq!(config.logging.record_text, true);
}

/// Verify that when the frontend sends `null` for an Option<String> field,
/// it correctly deserializes to None.
#[test]
fn test_null_deserializes_to_none() {
    let json = serde_json::json!({
        "asr": { "provider": "mock" },
        "llm": {
            "enabled": false,
            "provider": "mock",
            "prompt": null
        },
        "pipeline": { "performance": "low", "plugins": [] },
        "injector": { "method": "clipboard" },
        "audio": {},
        "history": {},
        "shortcut": {},
        "overlay": {},
        "ui": {}
    });

    let config: AppConfig = serde_json::from_value(json).expect("null→Option<String> should work");
    assert_eq!(config.llm.prompt, None);

    let json2 = serde_json::json!({
        "asr": { "provider": "mock" },
        "llm": {
            "enabled": false,
            "provider": "mock",
            "prompt": ""
        },
        "pipeline": { "performance": "low", "plugins": [] },
        "injector": { "method": "clipboard" },
        "audio": {},
        "history": {},
        "shortcut": {},
        "overlay": {},
        "ui": {}
    });
    let config2: AppConfig = serde_json::from_value(json2).unwrap();
    assert_eq!(config2.llm.prompt.as_deref(), Some(""));
}
