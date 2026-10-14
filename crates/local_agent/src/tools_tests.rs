use super::*;

fn client_tool(kind: ToolKind, arguments: &str) -> Tool {
    match parse_call(kind, arguments, false) {
        Ok(ParsedCall::Client(tool)) => *tool,
        Ok(ParsedCall::Todos(_)) => panic!("expected a client tool"),
        Err(error) => panic!("parse failed: {error}"),
    }
}

#[test]
fn shell_command_risk_maps_to_warp_flags() {
    let Tool::RunShellCommand(call) = client_tool(
        ToolKind::RunShellCommand,
        r#"{"command":"ls -la","risk":"read_only"}"#,
    ) else {
        panic!("wrong tool");
    };
    assert_eq!(call.command, "ls -la");
    assert!(call.is_read_only);
    assert!(!call.is_risky);
    assert_eq!(call.risk_category, api::RiskCategory::ReadOnly as i32);
    assert!(call.wait_until_complete_value.is_none());

    let Tool::RunShellCommand(call) = client_tool(
        ToolKind::RunShellCommand,
        r#"{"command":"rm -rf build","risk":"risky","wait_until_complete":"false"}"#,
    ) else {
        panic!("wrong tool");
    };
    assert!(call.is_risky);
    assert_eq!(
        call.wait_until_complete_value,
        Some(tc::run_shell_command::WaitUntilCompleteValue::WaitUntilComplete(false))
    );
}

#[test]
fn missing_risk_is_treated_as_a_local_change() {
    let Tool::RunShellCommand(call) =
        client_tool(ToolKind::RunShellCommand, r#"{"command":"make"}"#)
    else {
        panic!("wrong tool");
    };
    assert!(!call.is_read_only);
    assert_eq!(
        call.risk_category,
        api::RiskCategory::NontrivialLocalChange as i32
    );
}

#[test]
fn read_files_accepts_common_shapes() {
    let Tool::ReadFiles(call) = client_tool(
        ToolKind::ReadFiles,
        r#"{"files":[{"path":"a.rs","start_line":"10","end_line":20},"b.rs"]}"#,
    ) else {
        panic!("wrong tool");
    };
    assert_eq!(call.files.len(), 2);
    assert_eq!(call.files[0].name, "a.rs");
    assert_eq!(
        call.files[0].line_ranges,
        vec![api::FileContentLineRange { start: 10, end: 20 }]
    );
    assert!(call.files[1].line_ranges.is_empty());

    let Tool::ReadFiles(call) = client_tool(ToolKind::ReadFiles, r#"{"path":"c.rs"}"#) else {
        panic!("wrong tool");
    };
    assert_eq!(call.files[0].name, "c.rs");
}

#[test]
fn double_encoded_arguments_are_accepted() {
    let Tool::Grep(call) = client_tool(ToolKind::Grep, r#""{\"queries\":\"fn main\"}""#) else {
        panic!("wrong tool");
    };
    assert_eq!(call.queries, vec!["fn main"]);
    assert_eq!(call.path, ".");
}

#[test]
fn edits_require_a_search_string() {
    let error = parse_call(
        ToolKind::EditFiles,
        r#"{"edits":[{"file_path":"a.rs","search":"","replace":"x"}]}"#,
        false,
    )
    .err()
    .unwrap();
    assert!(error.contains("create_file"), "{error}");

    let Tool::ApplyFileDiffs(call) = client_tool(
        ToolKind::EditFiles,
        r#"{"summary":"Rename","edits":[{"path":"a.rs","old_string":"foo","new_string":"bar"}]}"#,
    ) else {
        panic!("wrong tool");
    };
    assert_eq!(call.summary, "Rename");
    assert_eq!(call.diffs[0].search, "foo");
    assert_eq!(call.diffs[0].replace, "bar");
}

#[test]
fn invalid_json_is_reported_to_the_model() {
    let error = parse_call(ToolKind::RunShellCommand, "{\"command\": ", false)
        .err()
        .unwrap();
    assert!(error.contains("not valid JSON"), "{error}");
    let error = parse_call(ToolKind::RunShellCommand, "{}", false)
        .err()
        .unwrap();
    assert!(error.contains("command"), "{error}");
}

#[test]
fn todo_updates_create_then_update_the_list() {
    let arguments = r#"{"todos":[
        {"id":"1","title":"Read code","status":"completed"},
        {"id":"2","title":"Fix bug","status":"pending"}
    ]}"#;
    let Ok(ParsedCall::Todos(created)) = parse_call(ToolKind::UpdateTodos, arguments, false) else {
        panic!("expected todo operations");
    };
    assert!(matches!(
        &created[..],
        [TodoOperation::CreateTodoList(list), TodoOperation::MarkTodosCompleted(done)]
            if list.initial_todos.len() == 2
                && list.initial_todos[0].title == "Read code"
                && done.todo_ids == vec!["1".to_owned()]
    ));

    let Ok(ParsedCall::Todos(updated)) = parse_call(ToolKind::UpdateTodos, arguments, true) else {
        panic!("expected todo operations");
    };
    assert!(matches!(
        &updated[..],
        [TodoOperation::MarkTodosCompleted(done), TodoOperation::UpdatePendingTodos(pending)]
            if done.todo_ids == vec!["1".to_owned()]
                && pending.updated_pending_todos.len() == 1
    ));
}

#[test]
fn tools_follow_client_capabilities() {
    let tools = available_tools(&[
        api::ToolType::RunShellCommand as i32,
        api::ToolType::ApplyFileDiffs as i32,
    ]);
    assert_eq!(
        tools,
        vec![
            ToolKind::RunShellCommand,
            ToolKind::EditFiles,
            ToolKind::CreateFile,
            ToolKind::DeleteFiles,
            ToolKind::UpdateTodos,
        ]
    );
}

#[test]
fn recorded_calls_round_trip() {
    let record = RecordedCall {
        id: Some("call_1".to_owned()),
        name: "update_todos".to_owned(),
        arguments: "{}".to_owned(),
    };
    assert_eq!(RecordedCall::decode(&record.encode()), Some(record));
    assert_eq!(RecordedCall::decode("opaque server data"), None);
}
