use super::*;
use crate::tools::RecordedCall;

fn message(kind: api::message::Message) -> api::Message {
    api::Message {
        id: uuid::Uuid::new_v4().to_string(),
        message: Some(kind),
        ..Default::default()
    }
}

fn user(text: &str) -> api::Message {
    message(api::message::Message::UserQuery(api::message::UserQuery {
        query: text.to_owned(),
        ..Default::default()
    }))
}

fn output(text: &str) -> api::Message {
    message(api::message::Message::AgentOutput(
        api::message::AgentOutput {
            text: text.to_owned(),
        },
    ))
}

fn shell_call(id: &str, command: &str) -> api::Message {
    let mut message = message(api::message::Message::ToolCall(api::message::ToolCall {
        tool_call_id: id.to_owned(),
        tool: Some(api::message::tool_call::Tool::RunShellCommand(
            api::message::tool_call::RunShellCommand {
                command: command.to_owned(),
                ..Default::default()
            },
        )),
    }));
    message.server_message_data = RecordedCall {
        id: None,
        name: "run_shell_command".to_owned(),
        arguments: format!(r#"{{"command":"{command}","risk":"read_only"}}"#),
    }
    .encode();
    message
}

fn shell_result(id: &str, output: &str) -> api::Message {
    message(api::message::Message::ToolCallResult(
        api::message::ToolCallResult {
            tool_call_id: id.to_owned(),
            context: None,
            result: Some(api::message::tool_call_result::Result::RunShellCommand(
                api::RunShellCommandResult {
                    result: Some(api::run_shell_command_result::Result::CommandFinished(
                        api::ShellCommandFinished {
                            output: output.to_owned(),
                            ..Default::default()
                        },
                    )),
                    ..Default::default()
                },
            )),
        },
    ))
}

#[test]
fn tool_calls_replay_with_the_models_arguments() {
    let history = build(
        &[
            user("list files"),
            output("Let me look."),
            shell_call("c1", "ls"),
            shell_result("c1", "a.txt"),
            output("There is one file."),
        ],
        1_000,
    );
    assert_eq!(history.len(), 4);
    assert_eq!(history[0], ChatMessage::user("list files"));
    let ChatMessage::Assistant {
        content,
        tool_calls,
    } = &history[1]
    else {
        panic!("expected assistant message");
    };
    assert_eq!(content.as_deref(), Some("Let me look."));
    assert_eq!(tool_calls[0].function.name, "run_shell_command");
    assert!(tool_calls[0].function.arguments.contains("read_only"));
    assert!(
        matches!(&history[2], ChatMessage::Tool { tool_call_id, content } if tool_call_id == "c1" && content.contains("a.txt"))
    );
    assert_eq!(history[3], ChatMessage::assistant("There is one file."));
}

#[test]
fn unanswered_calls_get_synthesized_results() {
    let history = build(
        &[
            user("delete the build dir"),
            shell_call("c1", "rm -rf build"),
            user("actually, don't"),
        ],
        1_000,
    );
    assert!(
        matches!(&history[2], ChatMessage::Tool { tool_call_id, content } if tool_call_id == "c1" && content.contains("not run"))
    );
    assert_eq!(history[3], ChatMessage::user("actually, don't"));

    let trailing = build(&[user("go"), shell_call("c2", "ls")], 1_000);
    assert!(
        matches!(trailing.last(), Some(ChatMessage::Tool { tool_call_id, .. }) if tool_call_id == "c2")
    );
}

#[test]
fn orphan_results_are_dropped() {
    let history = build(&[user("hi"), shell_result("missing", "x")], 1_000);
    assert_eq!(history, vec![ChatMessage::user("hi")]);
}

#[test]
fn budget_drops_old_turns_then_old_outputs() {
    let long = "x".repeat(2_000);
    let history = build(
        &[
            user("first"),
            output(&long),
            user("second"),
            shell_call("c1", "ls"),
            shell_result("c1", &long),
            output("done"),
        ],
        10_000,
    );
    let trimmed = fit_to_budget(history.clone(), 3_000);
    assert_eq!(trimmed[0], ChatMessage::user("second"));
    assert_eq!(trimmed.len(), 4, "only the oldest turn is dropped");

    let trimmed = fit_to_budget(history, 500);
    assert_eq!(trimmed[0], ChatMessage::user("second"));
    assert!(
        matches!(&trimmed[2], ChatMessage::Tool { content, .. } if content.contains("omitted"))
    );
}
