use super::*;

fn chunk(json: serde_json::Value) -> ChatChunk {
    serde_json::from_value(json).unwrap()
}

#[test]
fn sse_decoder_handles_split_lines_and_comments() {
    let mut decoder = SseDecoder::default();
    assert!(decoder.push(b"data: {\"a\":").is_empty());
    assert_eq!(decoder.push(b"1}\r\n\r\n: keep-alive\n"), vec!["{\"a\":1}"]);
    assert_eq!(decoder.push(b"data: [DONE]\n"), vec!["[DONE]"]);
    assert!(decoder.push(b"event: ping\n").is_empty());
    assert_eq!(decoder.push(b"data:tail"), Vec::<String>::new());
    assert_eq!(decoder.finish().as_deref(), Some("tail"));
}

#[test]
fn think_splitter_handles_tags_across_chunks() {
    let mut splitter = ThinkSplitter::default();
    let mut pieces = Vec::new();
    for part in ["<thi", "nk>plan it", "</th", "ink>ls -la", " <"] {
        pieces.extend(splitter.push(part));
    }
    pieces.extend(splitter.finish());
    assert_eq!(
        pieces,
        vec![
            Piece::Reasoning("plan it".to_owned()),
            Piece::Text("ls -la".to_owned()),
            Piece::Text(" ".to_owned()),
            // Held back as a possible tag start until the stream ends.
            Piece::Text("<".to_owned()),
        ]
    );
}

#[test]
fn assembler_streams_text_and_reasoning() {
    let mut assembler = StreamAssembler::default();
    let mut events = assembler.push_chunk(chunk(serde_json::json!({
        "choices": [{ "delta": { "reasoning": "thinking" } }]
    })));
    events.extend(assembler.push_chunk(chunk(serde_json::json!({
        "choices": [{ "delta": { "content": "Hello" }, "finish_reason": "stop" }],
        "usage": { "prompt_tokens": 10, "completion_tokens": 2 }
    }))));
    let (rest, output) = assembler.finish(true);
    assert!(rest.is_empty());
    assert_eq!(
        events,
        vec![
            StreamEvent::Reasoning("thinking".to_owned()),
            StreamEvent::Text("Hello".to_owned()),
        ]
    );
    assert_eq!(output.text, "Hello");
    assert_eq!(output.reasoning, "thinking");
    assert_eq!(output.finish_reason, FinishReason::Stop);
    assert_eq!(output.usage.unwrap().prompt_tokens, 10);
}

#[test]
fn assembler_joins_tool_call_fragments() {
    let mut assembler = StreamAssembler::default();
    for json in [
        serde_json::json!({ "choices": [{ "delta": { "tool_calls": [
            { "index": 0, "id": "call_a", "function": { "name": "run_shell_command", "arguments": "" } }
        ] } }] }),
        serde_json::json!({ "choices": [{ "delta": { "tool_calls": [
            { "index": 0, "function": { "arguments": "{\"command\":" } },
            { "index": 1, "function": { "name": "read_files", "arguments": "{}" } }
        ] } }] }),
        serde_json::json!({ "choices": [{ "delta": { "tool_calls": [
            { "index": 0, "function": { "arguments": "\"ls\"}" } }
        ] }, "finish_reason": "stop" }] }),
    ] {
        assert!(assembler.push_chunk(chunk(json)).is_empty());
    }
    let (_, output) = assembler.finish(true);
    assert_eq!(output.finish_reason, FinishReason::ToolCalls);
    assert_eq!(
        output.tool_calls,
        vec![
            ToolCall::new("call_a", "run_shell_command", "{\"command\":\"ls\"}"),
            ToolCall::new("call_1", "read_files", "{}"),
        ]
    );
}

#[test]
fn truncated_stream_is_reported() {
    let mut assembler = StreamAssembler::default();
    assembler.push_chunk(chunk(serde_json::json!({
        "choices": [{ "delta": { "content": "partial" } }]
    })));
    let (_, output) = assembler.finish(false);
    assert!(matches!(output.finish_reason, FinishReason::Other(_)));
}
