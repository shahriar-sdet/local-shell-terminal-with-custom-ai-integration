use super::*;

#[test]
fn truncate_middle_keeps_head_and_tail() {
    let text = format!("{}{}", "a".repeat(100), "z".repeat(100));
    let truncated = truncate_middle(&text, 50);
    assert!(truncated.starts_with("aaaaa"));
    assert!(truncated.ends_with("zzzzz"));
    assert!(truncated.contains("characters omitted"));
    assert_eq!(truncate_middle("short", 50), "short");
}

#[test]
fn truncate_middle_respects_char_boundaries() {
    let text = "é".repeat(100);
    let truncated = truncate_middle(&text, 33);
    assert!(truncated.contains("omitted"));
}

#[test]
fn shell_results_include_exit_code_and_output() {
    let result = ToolResult::RunShellCommand(api::RunShellCommandResult {
        command: "ls".to_owned(),
        result: Some(api::run_shell_command_result::Result::CommandFinished(
            api::ShellCommandFinished {
                output: "Cargo.toml\nsrc\n".to_owned(),
                exit_code: 0,
                command_id: "cmd-1".to_owned(),
                ..Default::default()
            },
        )),
        ..Default::default()
    });
    let text = tool_result(Some(&result), 1_000);
    assert!(text.contains("exited with code 0"));
    assert!(text.contains("Cargo.toml"));
}

#[test]
fn grep_results_list_files_and_lines() {
    use api::grep_result::success::GrepFileMatch;
    use api::grep_result::success::grep_file_match::GrepLineMatch;
    let result = ToolResult::Grep(api::GrepResult {
        result: Some(api::grep_result::Result::Success(
            api::grep_result::Success {
                matched_files: vec![GrepFileMatch {
                    file_path: "src/main.rs".to_owned(),
                    matched_lines: vec![
                        GrepLineMatch { line_number: 3 },
                        GrepLineMatch { line_number: 9 },
                    ],
                }],
            },
        )),
    });
    assert_eq!(
        tool_result(Some(&result), 1_000),
        "1 files matched:\nsrc/main.rs: lines 3, 9\n"
    );
}

#[test]
fn user_query_renders_attachments_before_the_question() {
    let mut query = api::message::UserQuery {
        query: "why did this fail?".to_owned(),
        ..Default::default()
    };
    query.referenced_attachments.insert(
        "block-1".to_owned(),
        api::Attachment {
            value: Some(api::attachment::Value::ExecutedShellCommand(
                api::ExecutedShellCommand {
                    command: "cargo build".to_owned(),
                    output: "error[E0425]".to_owned(),
                    exit_code: 101,
                    ..Default::default()
                },
            )),
        },
    );
    let parts = user_query(&query, 1_000);
    let [ContentPart::Text { text }] = parts.as_slice() else {
        panic!("expected a single text part");
    };
    let attachment = text.find("cargo build").unwrap();
    let question = text.find("why did this fail?").unwrap();
    assert!(attachment < question);
    assert!(text.contains("exit code 101"));
}
