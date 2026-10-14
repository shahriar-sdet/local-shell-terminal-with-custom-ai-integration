use super::*;

#[test]
fn default_config_points_at_lm_studio_and_needs_a_model() {
    let config = LocalAiConfig::default();
    assert_eq!(config.provider, ProviderKind::LmStudio);
    assert_eq!(
        config.parsed_base_url().unwrap().as_str(),
        "http://localhost:1234/v1/"
    );
    assert!(!config.is_configured());
}

#[test]
fn base_url_is_normalized_for_joining() {
    let config = LocalAiConfig {
        base_url: " https://api.openai.com/v1 ".to_owned(),
        ..Default::default()
    };
    let url = config.parsed_base_url().unwrap();
    assert_eq!(
        url.join("chat/completions").unwrap().as_str(),
        "https://api.openai.com/v1/chat/completions"
    );
}

#[test]
fn rejects_non_http_endpoints() {
    for base_url in [
        "",
        "localhost:1234",
        "file:///tmp/socket",
        "ftp://example.com/v1",
    ] {
        let config = LocalAiConfig {
            base_url: base_url.to_owned(),
            ..Default::default()
        };
        assert!(
            config.parsed_base_url().is_none(),
            "{base_url} should be rejected"
        );
    }
}

#[test]
fn tasks_fall_back_to_the_agent_model() {
    let mut config = LocalAiConfig::default();
    config.agent.model = "qwen3-coder-30b".to_owned();
    config.suggestions.model = "qwen3-4b".to_owned();
    assert!(config.is_configured());
    assert_eq!(config.model_for(TaskKind::Suggestions), "qwen3-4b");
    assert_eq!(
        config.model_for(TaskKind::CommandGeneration),
        "qwen3-coder-30b"
    );
}

#[test]
fn partial_toml_uses_defaults_for_missing_fields() {
    let config: LocalAiConfig = serde_json::from_value(serde_json::json!({
        "provider": "open_ai_compatible",
        "base_url": "https://api.openai.com/v1",
        "agent": { "model": "gpt-5" }
    }))
    .unwrap();
    assert_eq!(config.provider, ProviderKind::OpenAiCompatible);
    assert_eq!(config.agent.model, "gpt-5");
    assert_eq!(config.agent.context_length, 32_768);
    assert_eq!(
        config.suggestions,
        TaskProfile::preset(TaskKind::Suggestions)
    );
}
