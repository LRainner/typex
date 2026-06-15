use super::*;

/// Simulate the exact data flow: frontend JSON → Rust AppConfig → TOML → verify
#[test]
fn test_llm_config_roundtrip_from_json() {
    // This is what the frontend sends via invoke('save_config', { config: ... })
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

    // Step 1: Deserialize from JSON (mimics Tauri IPC)
    let config: AppConfig = serde_json::from_value(json).expect("JSON → AppConfig should work");

    // Step 2: Verify fields
    assert_eq!(config.llm.enabled, true);
    assert_eq!(config.llm.provider, "openai-compatible");
    assert_eq!(
        config.llm.endpoint.as_deref(),
        Some("https://api.openai.com/v1")
    );
    assert_eq!(config.llm.model.as_deref(), Some("gpt-4o-mini"));
    assert_eq!(config.llm.api_key.as_deref(), Some("sk-test-key-12345"));
    assert_eq!(
        config.llm.prompt.as_deref(),
        Some("You are a helpful assistant.")
    );

    // Step 3: Serialize to TOML (mimics config.save())
    let toml_str = toml::to_string_pretty(&config).expect("AppConfig → TOML should work");
    println!("=== TOML OUTPUT ===\n{}", toml_str);

    // Step 4: Verify TOML contains the LLM fields
    assert!(
        toml_str.contains("endpoint"),
        "TOML should contain endpoint"
    );
    assert!(toml_str.contains("api_key"), "TOML should contain api_key");
    assert!(toml_str.contains("model"), "TOML should contain model");

    // Step 5: Round-trip: TOML → AppConfig → verify same values
    let config2: AppConfig = toml::from_str(&toml_str).expect("TOML → AppConfig should work");
    assert_eq!(
        config2.llm.endpoint.as_deref(),
        Some("https://api.openai.com/v1")
    );
    assert_eq!(config2.llm.model.as_deref(), Some("gpt-4o-mini"));
    assert_eq!(config2.llm.api_key.as_deref(), Some("sk-test-key-12345"));
}

/// Verify that Option::None fields are omitted from TOML (expected TOML behavior)
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

    let config: AppConfig = serde_json::from_value(json).unwrap();
    let toml_str = toml::to_string_pretty(&config).unwrap();
    println!("=== TOML WITH NULLS ===\n{}", toml_str);

    // Option::None fields should NOT appear in TOML
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

    // But empty Some("") prompt SHOULD appear in TOML (it's Some, not None)
    assert!(
        toml_str.contains("prompt"),
        "Empty prompt string should appear in TOML"
    );
    assert!(
        toml_str.contains("enabled"),
        "enabled bool should appear in TOML"
    );
}

/// Verify that when the frontend sends `null` for an Option<String> field,
/// it correctly deserializes to None (rather than failing).
/// This was the root cause of config save failures before `prompt` was
/// changed from String to Option<String>.
#[test]
fn test_null_deserializes_to_none() {
    // Simulate what happens when prompt textarea is empty:
    // currentConfig.llm.prompt = document.getElementById('llm-prompt').value || '';
    // Now sends "prompt": "" (since JS fix changed || null to || '')
    // But even if old buggy code sends null, it should still work now.
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
    assert_eq!(
        config.llm.prompt, None,
        "null should become None for Option<String>"
    );
    println!("✅ null→Option<String> correctly deserialized to None");

    // Also test: empty string → Some("")
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
    println!("✅ empty string→Option<String> correctly deserialized to Some(\"\")");
}
