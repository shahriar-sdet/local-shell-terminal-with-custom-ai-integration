use super::*;

#[test]
fn json_answers_are_found_inside_prose_and_fences() {
    let fenced = "Sure!\n```json\n{\"commands\":[{\"command\":\"ls -la\"}]}\n```";
    let parsed: GeneratedCommands = parse_json_answer(fenced).unwrap();
    assert_eq!(parsed.commands[0].command, "ls -la");
    assert!(parsed.commands[0].parameters.is_empty());

    let plain: Vec<String> = parse_json_answer("[\"a\", \"b\"]").unwrap();
    assert_eq!(plain, vec!["a", "b"]);

    assert!(parse_json_answer::<GeneratedCommands>("no json here").is_none());
}

#[test]
fn parameters_accept_name_or_id() {
    let parsed: GeneratedCommands = parse_json_answer(
        r#"{"commands":[{"command":"kill {{pid}}","parameters":[{"name":"pid","description":"Process id"}]}]}"#,
    )
    .unwrap();
    assert_eq!(parsed.commands[0].parameters[0].id, "pid");
}

#[test]
fn environment_line_describes_os_and_shell() {
    let context = WarpAiExecutionContext {
        os: ai_types::WarpAiOsContext {
            category: Some("macOS".to_owned()),
            distribution: None,
        },
        shell_name: "zsh".to_owned(),
        shell_version: Some("5.9".to_owned()),
    };
    assert_eq!(
        environment_line(Some(&context)),
        "The user is on macOS using the zsh 5.9 shell."
    );
}

#[test]
fn code_fences_around_answers_are_removed() {
    assert_eq!(strip_code_fence("```\nfeat: add x\n```"), "feat: add x");
    assert_eq!(
        strip_code_fence("```text\nfix: y\n\nbody\n```"),
        "fix: y\n\nbody"
    );
    assert_eq!(strip_code_fence("  plain title "), "plain title");
}
