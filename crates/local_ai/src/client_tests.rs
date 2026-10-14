use futures::executor::block_on;

use super::*;
use crate::types::ToolCall;

fn config(provider: ProviderKind, base_url: &str) -> LocalAiConfig {
    let mut config = LocalAiConfig {
        provider,
        base_url: base_url.to_owned(),
        ..Default::default()
    };
    config.agent.model = "qwen3-coder".to_owned();
    config
}

#[test]
fn request_uses_task_profile() {
    let config = config(ProviderKind::LmStudio, "http://localhost:1234/v1");
    let client = LocalAiClient::new(&config).unwrap();
    let request = client.request_for_task(
        &config,
        TaskKind::Suggestions,
        vec![ChatMessage::user("hi")],
        Vec::new(),
    );
    assert_eq!(request.model, "qwen3-coder");
    assert_eq!(request.max_tokens, Some(128));
    assert_eq!(request.max_completion_tokens, None);
    assert_eq!(request.reasoning_effort.as_deref(), Some("none"));
    assert_eq!(request.temperature, Some(0.2));

    let json = serde_json::to_value(&request).unwrap();
    assert!(json.get("tools").is_none(), "empty tool lists are omitted");
    assert!(json.get("stream_options").is_none());
}

#[test]
fn openai_uses_max_completion_tokens() {
    let config = config(ProviderKind::OpenAiCompatible, "https://api.openai.com/v1");
    let client = LocalAiClient::new(&config).unwrap();
    let request = client.request_for_task(&config, TaskKind::Utility, Vec::new(), Vec::new());
    assert_eq!(request.max_tokens, None);
    assert_eq!(request.max_completion_tokens, Some(256));
}

#[test]
fn root_url_strips_version_segment() {
    let config = config(ProviderKind::LmStudio, "http://localhost:1234/v1");
    let client = LocalAiClient::new(&config).unwrap();
    assert_eq!(
        client.root_url("api/v1/models").unwrap().as_str(),
        "http://localhost:1234/api/v1/models"
    );
    assert_eq!(
        client.api_url("chat/completions").unwrap().as_str(),
        "http://localhost:1234/v1/chat/completions"
    );
}

#[test]
fn invalid_base_url_is_rejected() {
    let config = config(ProviderKind::LmStudio, "localhost:1234");
    assert!(matches!(
        LocalAiClient::new(&config),
        Err(LocalAiError::InvalidUrl(_))
    ));
}

#[test]
fn rejected_parameters_are_dropped_one_at_a_time() {
    let mut request = ChatRequest {
        model: "m".to_owned(),
        messages: Vec::new(),
        tools: Vec::new(),
        temperature: Some(0.2),
        max_tokens: Some(10),
        max_completion_tokens: None,
        reasoning_effort: Some("low".to_owned()),
        stream: true,
        stream_options: None,
    };
    assert!(drop_rejected_parameter(
        &mut request,
        "Unsupported parameter: 'reasoning_effort'"
    ));
    assert_eq!(request.reasoning_effort, None);
    assert!(drop_rejected_parameter(
        &mut request,
        "Unsupported value: 'temperature' does not support 0.2"
    ));
    assert_eq!(request.temperature, None);
    assert!(drop_rejected_parameter(
        &mut request,
        "'max_tokens' is not supported with this model. Use 'max_completion_tokens' instead."
    ));
    assert_eq!(request.max_tokens, None);
    assert_eq!(request.max_completion_tokens, Some(10));
    assert!(!drop_rejected_parameter(&mut request, "model not found"));
}

#[test]
fn error_bodies_are_classified() {
    assert_eq!(
        LocalAiError::from_status(401, "{}"),
        LocalAiError::Unauthorized { status: 401 }
    );
    assert!(matches!(
        LocalAiError::from_status(
            400,
            r#"{"error":{"message":"This model's maximum context length is 4096 tokens","code":"context_length_exceeded"}}"#
        ),
        LocalAiError::ContextLengthExceeded(_)
    ));
    assert_eq!(
        LocalAiError::from_status(404, r#"{"error":"Model not found"}"#),
        LocalAiError::Http {
            status: 404,
            message: "Model not found".to_owned()
        }
    );
}

fn decode(parts: &[&str]) -> Vec<Result<StreamEvent, LocalAiError>> {
    let chunks: Vec<reqwest::Result<Bytes>> = parts
        .iter()
        .map(|part| Ok(Bytes::from(part.to_string())))
        .collect();
    block_on(decode_stream(futures::stream::iter(chunks)).collect())
}

#[test]
fn stream_decoding_end_to_end() {
    let events = decode(&[
        "data: {\"choices\":[{\"delta\":{\"content\":\"<think>hm</think>Run \"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\"function\":{\"name\":\"run_shell_command\",\"arguments\":\"{}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
        "data: [DONE]\n\n",
    ]);
    let events: Vec<_> = events.into_iter().map(Result::unwrap).collect();
    assert_eq!(events[0], StreamEvent::Reasoning("hm".to_owned()));
    assert_eq!(events[1], StreamEvent::Text("Run ".to_owned()));
    let StreamEvent::Finished(output) = &events[2] else {
        panic!("expected the stream to finish");
    };
    assert_eq!(output.finish_reason, FinishReason::ToolCalls);
    assert_eq!(
        output.tool_calls,
        vec![ToolCall::new("c1", "run_shell_command", "{}")]
    );
}

#[test]
fn stream_error_payload_is_surfaced() {
    let events = decode(&["data: {\"error\":{\"message\":\"Model crashed\"}}\n\n"]);
    assert_eq!(events.len(), 1);
    assert!(
        matches!(&events[0], Err(LocalAiError::Http { message, .. }) if message == "Model crashed")
    );
}
