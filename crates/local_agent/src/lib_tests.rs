use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::Mutex;
use std::thread;

use futures::executor::block_on;
use local_ai::ProviderKind;
use serde_json::{Value, json};
use warp_multi_agent_api::request::input::user_inputs::{UserInput, user_input};
use warp_multi_agent_api::request::input::{self as req, Type as InputType};
use warp_multi_agent_api::response_event::Type as EventType;

use super::*;

/// An OpenAI-compatible server on localhost that answers each request with the next canned
/// stream of chunks and records the request bodies.
struct FakeModel {
    base_url: String,
    requests: Arc<Mutex<Vec<Value>>>,
}

fn fake_model(responses: Vec<Vec<Value>>) -> FakeModel {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = requests.clone();
    thread::spawn(move || {
        for chunks in responses {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut content_length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    content_length = value.trim().parse().unwrap();
                }
            }
            let mut body = vec![0; content_length];
            reader.read_exact(&mut body).unwrap();
            recorded
                .lock()
                .unwrap()
                .push(serde_json::from_slice(&body).unwrap());
            let mut sse = String::new();
            for chunk in chunks {
                sse.push_str(&format!("data: {chunk}\n\n"));
            }
            sse.push_str("data: [DONE]\n\n");
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n{sse}",
                sse.len()
            )
            .unwrap();
        }
    });
    FakeModel {
        base_url: format!("http://127.0.0.1:{port}/v1"),
        requests,
    }
}

fn config(base_url: &str) -> LocalAiConfig {
    let mut config = LocalAiConfig {
        provider: ProviderKind::OpenAiCompatible,
        base_url: base_url.to_owned(),
        ..Default::default()
    };
    config.agent.model = "test-model".to_owned();
    config.agent.context_length = 8_192;
    config
}

fn run_engine(model: &FakeModel, request: api::Request) -> Vec<api::ResponseEvent> {
    let config = config(&model.base_url);
    let client = Arc::new(LocalAiClient::new(&config).unwrap());
    block_on(generate_multi_agent_output(client, config, request).collect())
}

fn user_query_request(query: &str) -> api::Request {
    api::Request {
        input: Some(api::request::Input {
            context: Some(api::InputContext {
                directory: Some(api::input_context::Directory {
                    pwd: "/tmp/project".to_owned(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            r#type: Some(InputType::UserInputs(req::UserInputs {
                inputs: vec![UserInput {
                    input: Some(user_input::Input::UserQuery(req::UserQuery {
                        query: query.to_owned(),
                        ..Default::default()
                    })),
                }],
            })),
        }),
        settings: Some(api::request::Settings {
            supported_tools: vec![
                api::ToolType::RunShellCommand as i32,
                api::ToolType::ReadFiles as i32,
            ],
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn actions(event: &api::ResponseEvent) -> Vec<Action> {
    match &event.r#type {
        Some(EventType::ClientActions(actions)) => actions
            .actions
            .iter()
            .filter_map(|action| action.action.clone())
            .collect(),
        _ => Vec::new(),
    }
}

fn finish_reason(event: &api::ResponseEvent) -> Option<&stream_finished::Reason> {
    match &event.r#type {
        Some(EventType::Finished(finished)) => finished.reason.as_ref(),
        _ => None,
    }
}

fn added_messages(events: &[api::ResponseEvent]) -> Vec<api::Message> {
    events
        .iter()
        .flat_map(actions)
        .filter_map(|action| match action {
            Action::AddMessagesToTask(add) => Some(add.messages),
            _ => None,
        })
        .flatten()
        .collect()
}

#[test]
fn new_conversation_streams_text() {
    let model = fake_model(vec![vec![
        json!({"choices": [{"delta": {"reasoning": "thinking"}}]}),
        json!({"choices": [{"delta": {"content": "Hello"}}]}),
        json!({"choices": [{"delta": {"content": " there"}, "finish_reason": "stop"}],
               "usage": {"prompt_tokens": 1024, "completion_tokens": 2}}),
    ]]);
    let events = run_engine(&model, user_query_request("hi"));

    assert!(matches!(events[0].r#type, Some(EventType::Init(_))));
    let setup = actions(&events[1]);
    let Action::CreateTask(create) = &setup[0] else {
        panic!("a new conversation creates its root task first");
    };
    let root_id = create.task.as_ref().unwrap().id.clone();
    let Action::AddMessagesToTask(echo) = &setup[1] else {
        panic!("the user query is echoed into the task");
    };
    assert_eq!(echo.task_id, root_id);
    assert!(matches!(
        &echo.messages[0].message,
        Some(api::message::Message::UserQuery(query)) if query.query == "hi"
    ));

    let messages = added_messages(&events);
    assert!(messages.iter().any(|message| matches!(
        &message.message,
        Some(api::message::Message::AgentReasoning(reasoning)) if reasoning.reasoning == "thinking"
    )));
    let text_message = messages
        .iter()
        .find(|message| matches!(message.message, Some(api::message::Message::AgentOutput(_))))
        .unwrap();
    let appended = events.iter().flat_map(actions).any(|action| {
        matches!(action, Action::AppendToMessageContent(append)
            if append.message.as_ref().unwrap().id == text_message.id)
    });
    assert!(appended, "later text deltas append to the same message");

    let last = events.last().unwrap();
    assert!(matches!(
        finish_reason(last),
        Some(stream_finished::Reason::Done(_))
    ));
    let Some(EventType::Finished(finished)) = &last.r#type else {
        unreachable!();
    };
    let usage = finished.conversation_usage_metadata.as_ref().unwrap();
    assert_eq!(usage.context_window_usage, 0.125);

    let requests = model.requests.lock().unwrap();
    let body = &requests[0];
    assert_eq!(body["model"], "test-model");
    assert_eq!(body["stream"], true);
    assert!(
        body["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("/tmp/project")
    );
    assert_eq!(
        body["messages"][1],
        json!({"role": "user", "content": "hi"})
    );
    let tool_names: Vec<&str> = body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["function"]["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        tool_names,
        vec!["run_shell_command", "read_files", "update_todos"]
    );
}

#[test]
fn tool_calls_are_handed_to_the_client() {
    let model = fake_model(vec![vec![json!({"choices": [{
        "delta": {"tool_calls": [{"index": 0, "id": "c1", "function": {
            "name": "run_shell_command",
            "arguments": "{\"command\":\"ls\",\"risk\":\"read_only\"}"
        }}]},
        "finish_reason": "tool_calls"
    }]})]]);
    let events = run_engine(&model, user_query_request("what's here?"));

    let call = added_messages(&events)
        .into_iter()
        .find(|message| matches!(message.message, Some(api::message::Message::ToolCall(_))))
        .expect("a tool call message");
    let Some(api::message::Message::ToolCall(tool_call)) = &call.message else {
        unreachable!();
    };
    assert_eq!(tool_call.tool_call_id, "c1");
    assert!(matches!(
        &tool_call.tool,
        Some(api::message::tool_call::Tool::RunShellCommand(run)) if run.command == "ls" && run.is_read_only
    ));
    assert_eq!(
        RecordedCall::decode(&call.server_message_data)
            .unwrap()
            .name,
        "run_shell_command"
    );
    assert!(matches!(
        finish_reason(events.last().unwrap()),
        Some(stream_finished::Reason::Done(_))
    ));
    assert_eq!(
        model.requests.lock().unwrap().len(),
        1,
        "the client runs the tool next"
    );
}

#[test]
fn tool_results_continue_the_conversation() {
    let model = fake_model(vec![vec![json!({"choices": [{
        "delta": {"content": "There is one file."}, "finish_reason": "stop"
    }]})]]);
    let factory = MessageFactory {
        task_id: "root".to_owned(),
        request_id: "earlier".to_owned(),
    };
    let mut call = factory.new_message(api::message::Message::ToolCall(api::message::ToolCall {
        tool_call_id: "c1".to_owned(),
        tool: Some(api::message::tool_call::Tool::RunShellCommand(
            api::message::tool_call::RunShellCommand {
                command: "ls".to_owned(),
                ..Default::default()
            },
        )),
    }));
    call.server_message_data = RecordedCall {
        id: None,
        name: "run_shell_command".to_owned(),
        arguments: r#"{"command":"ls","risk":"read_only"}"#.to_owned(),
    }
    .encode();
    let query = factory.new_message(convert::text_query_message("what's here?".to_owned(), None));

    let mut request = user_query_request("unused");
    request.task_context = Some(api::request::TaskContext {
        tasks: vec![api::Task {
            id: "root".to_owned(),
            messages: vec![query, call],
            ..Default::default()
        }],
    });
    request.metadata = Some(api::request::Metadata {
        conversation_id: "conversation-1".to_owned(),
        ..Default::default()
    });
    request.input.as_mut().unwrap().r#type = Some(InputType::UserInputs(req::UserInputs {
        inputs: vec![UserInput {
            input: Some(user_input::Input::ToolCallResult(req::ToolCallResult {
                tool_call_id: "c1".to_owned(),
                result: Some(req::tool_call_result::Result::RunShellCommand(
                    api::RunShellCommandResult {
                        result: Some(api::run_shell_command_result::Result::CommandFinished(
                            api::ShellCommandFinished {
                                output: "a.txt\n".to_owned(),
                                ..Default::default()
                            },
                        )),
                        ..Default::default()
                    },
                )),
            })),
        }],
    }));

    let events = run_engine(&model, request);
    let Some(EventType::Init(init)) = &events[0].r#type else {
        panic!("expected init");
    };
    assert_eq!(init.conversation_id, "conversation-1");
    let setup = actions(&events[1]);
    assert_eq!(setup.len(), 1, "existing conversations don't create a task");
    let Action::AddMessagesToTask(echo) = &setup[0] else {
        panic!("the tool result is echoed into the task");
    };
    assert_eq!(echo.task_id, "root");
    assert!(matches!(
        echo.messages[0].message,
        Some(api::message::Message::ToolCallResult(_))
    ));

    let requests = model.requests.lock().unwrap();
    let messages = requests[0]["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 4);
    assert_eq!(messages[2]["role"], "assistant");
    assert_eq!(messages[2]["tool_calls"][0]["id"], "c1");
    assert_eq!(messages[3]["role"], "tool");
    assert_eq!(messages[3]["tool_call_id"], "c1");
    assert!(messages[3]["content"].as_str().unwrap().contains("a.txt"));
}

#[test]
fn invalid_calls_are_retried_with_feedback() {
    let model = fake_model(vec![
        vec![json!({"choices": [{
            "delta": {"tool_calls": [{"index": 0, "id": "bad", "function": {
                "name": "launch_rockets", "arguments": "{}"
            }}]},
            "finish_reason": "tool_calls"
        }]})],
        vec![json!({"choices": [{"delta": {"content": "Sorry."}, "finish_reason": "stop"}]})],
    ]);
    let events = run_engine(&model, user_query_request("go"));

    assert!(matches!(
        finish_reason(events.last().unwrap()),
        Some(stream_finished::Reason::Done(_))
    ));
    let requests = model.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    let retry = requests[1]["messages"].as_array().unwrap();
    let feedback = retry.last().unwrap();
    assert_eq!(feedback["role"], "tool");
    assert!(
        feedback["content"]
            .as_str()
            .unwrap()
            .contains("Unknown tool")
    );
    assert!(
        !added_messages(&events)
            .iter()
            .any(|message| matches!(message.message, Some(api::message::Message::ToolCall(_)))),
        "invalid calls never reach the client"
    );
}

#[test]
fn unreachable_endpoint_finishes_with_an_error() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let config = config(&format!("http://127.0.0.1:{port}/v1"));
    let client = Arc::new(LocalAiClient::new(&config).unwrap());
    let events: Vec<_> =
        block_on(generate_multi_agent_output(client, config, user_query_request("hi")).collect());
    assert!(matches!(
        finish_reason(events.last().unwrap()),
        Some(stream_finished::Reason::InternalError(error)) if error.message.contains("Could not reach")
    ));
}
