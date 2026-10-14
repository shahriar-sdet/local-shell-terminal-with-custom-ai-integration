//! Plain-text renderings of Warp messages for the model.

use std::fmt::Write as _;

use base64::Engine as _;
use local_ai::{ContentPart, ImageUrl};
use warp_multi_agent_api as api;
use warp_multi_agent_api::message::tool_call_result::Result as ToolResult;

/// Inserted where the middle of an over-long text was removed.
fn omission_marker(omitted: usize) -> String {
    format!("\n\n[... {omitted} characters omitted ...]\n\n")
}

/// Shortens `text` to roughly `max_chars` by removing its middle, which keeps both the start of
/// command output (what ran) and its end (errors, summaries).
pub(crate) fn truncate_middle(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.to_owned();
    }
    let head = floor_char_boundary(text, max_chars * 3 / 5);
    let tail = ceil_char_boundary(text, text.len() - (max_chars - max_chars * 3 / 5));
    format!(
        "{}{}{}",
        &text[..head],
        omission_marker(tail - head),
        &text[tail..]
    )
}

fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn ceil_char_boundary(text: &str, mut index: usize) -> usize {
    while !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

fn fenced(text: &str, max_chars: usize) -> String {
    let body = truncate_middle(text.trim_end(), max_chars);
    if body.is_empty() {
        "(no output)".to_owned()
    } else {
        format!("```\n{body}\n```")
    }
}

fn file_content(file: &api::FileContent, max_chars: usize) -> String {
    let range = file
        .line_range
        .map(|range| format!(" (lines {}-{})", range.start, range.end))
        .unwrap_or_default();
    format!(
        "{}{range}:\n{}",
        file.file_path,
        fenced(&file.content, max_chars)
    )
}

fn snapshot(snapshot: &api::LongRunningShellCommandSnapshot, max_chars: usize) -> String {
    let screen = if snapshot.is_alt_screen_active {
        " It is showing a full-screen interface."
    } else {
        ""
    };
    format!(
        "The command is still running (command_id: {}).{screen} Latest output:\n{}",
        snapshot.command_id,
        fenced(&snapshot.output, max_chars)
    )
}

fn finished(finished: &api::ShellCommandFinished, max_chars: usize) -> String {
    format!(
        "The command exited with code {} (command_id: {}). Output:\n{}",
        finished.exit_code,
        finished.command_id,
        fenced(&finished.output, max_chars)
    )
}

fn shell_error(error: &api::ShellCommandError) -> String {
    match error.r#type {
        Some(api::shell_command_error::Type::CommandNotFound(())) => {
            "No running command has that command_id; it may have already finished.".to_owned()
        }
        None => "The command could not be reached.".to_owned(),
    }
}

/// Renders a tool result for the model, truncating large outputs to `max_chars`.
#[allow(deprecated)]
pub(crate) fn tool_result(result: Option<&ToolResult>, max_chars: usize) -> String {
    let Some(result) = result else {
        return "The tool returned no result.".to_owned();
    };
    match result {
        ToolResult::RunShellCommand(result) => {
            use api::run_shell_command_result::Result as Run;
            match &result.result {
                Some(Run::CommandFinished(done)) => finished(done, max_chars),
                Some(Run::LongRunningCommandSnapshot(running)) => snapshot(running, max_chars),
                Some(Run::PermissionDenied(_)) => {
                    "Not run: the user's settings deny this command.".to_owned()
                }
                Some(Run::TerminalBusy(busy)) => format!(
                    "Not run: the terminal is busy with another command (command_id: {}). \
                     Wait for it, read its output, or send it input.",
                    busy.running_command_id
                ),
                None => format!(
                    "The command exited with code {}. Output:\n{}",
                    result.exit_code,
                    fenced(&result.output, max_chars)
                ),
            }
        }
        ToolResult::ReadFiles(result) => {
            use api::read_files_result::Result as Read;
            match &result.result {
                Some(Read::TextFilesSuccess(success)) => {
                    let mut text: Vec<String> = success
                        .files
                        .iter()
                        .map(|file| file_content(file, max_chars))
                        .collect();
                    text.extend(failed_reads(&success.failed_reads));
                    text.join("\n\n")
                }
                Some(Read::AnyFilesSuccess(success)) => {
                    let mut text: Vec<String> = success
                        .files
                        .iter()
                        .filter_map(|file| match &file.content {
                            Some(api::any_file_content::Content::TextContent(file)) => {
                                Some(file_content(file, max_chars))
                            }
                            Some(api::any_file_content::Content::BinaryContent(file)) => {
                                Some(format!(
                                    "{}: binary file ({} bytes), not shown.",
                                    file.file_path,
                                    file.data.len()
                                ))
                            }
                            None => None,
                        })
                        .collect();
                    text.extend(failed_reads(&success.failed_reads));
                    text.join("\n\n")
                }
                Some(Read::Error(error)) => format!("Reading failed: {}", error.message),
                None => "Reading returned nothing.".to_owned(),
            }
        }
        ToolResult::Grep(result) => match &result.result {
            Some(api::grep_result::Result::Success(success)) => {
                if success.matched_files.is_empty() {
                    return "No matches.".to_owned();
                }
                let mut text = format!("{} files matched:\n", success.matched_files.len());
                for file in &success.matched_files {
                    let lines: Vec<String> = file
                        .matched_lines
                        .iter()
                        .map(|line| line.line_number.to_string())
                        .collect();
                    let _ = writeln!(text, "{}: lines {}", file.file_path, lines.join(", "));
                }
                truncate_middle(&text, max_chars)
            }
            Some(api::grep_result::Result::Error(error)) => {
                format!("Search failed: {}", error.message)
            }
            None => "No matches.".to_owned(),
        },
        ToolResult::FileGlobV2(result) => match &result.result {
            Some(api::file_glob_v2_result::Result::Success(success)) => {
                if success.matched_files.is_empty() {
                    return "No files matched.".to_owned();
                }
                let files: Vec<&str> = success
                    .matched_files
                    .iter()
                    .map(|file| file.file_path.as_str())
                    .collect();
                let mut text = truncate_middle(&files.join("\n"), max_chars);
                if !success.warnings.is_empty() {
                    let _ = write!(text, "\nWarnings: {}", success.warnings);
                }
                text
            }
            Some(api::file_glob_v2_result::Result::Error(error)) => {
                format!("Finding files failed: {}", error.message)
            }
            None => "No files matched.".to_owned(),
        },
        ToolResult::FileGlob(result) => match &result.result {
            Some(api::file_glob_result::Result::Success(success)) => {
                truncate_middle(&success.matched_files, max_chars)
            }
            Some(api::file_glob_result::Result::Error(error)) => {
                format!("Finding files failed: {}", error.message)
            }
            None => "No files matched.".to_owned(),
        },
        ToolResult::ApplyFileDiffs(result) => match &result.result {
            Some(api::apply_file_diffs_result::Result::Success(success)) => {
                let mut text = "The changes were applied.".to_owned();
                for updated in &success.updated_files_v2 {
                    let Some(file) = &updated.file else {
                        continue;
                    };
                    if updated.was_edited_by_user {
                        let _ = write!(
                            text,
                            "\nThe user adjusted the change to {} before accepting. Current \
                             content:\n{}",
                            file.file_path,
                            fenced(&file.content, max_chars)
                        );
                    } else {
                        let _ = write!(text, "\nUpdated {}", file.file_path);
                    }
                }
                for deleted in &success.deleted_files {
                    let _ = write!(text, "\nDeleted {}", deleted.file_path);
                }
                text
            }
            Some(api::apply_file_diffs_result::Result::Error(error)) => format!(
                "The changes were not applied: {}. Re-read the file and retry with an exact \
                 search string.",
                error.message
            ),
            None => "The changes were not applied.".to_owned(),
        },
        ToolResult::WriteToLongRunningShellCommand(result) => {
            use api::write_to_long_running_shell_command_result::Result as Write;
            match &result.result {
                Some(Write::LongRunningCommandSnapshot(running)) => snapshot(running, max_chars),
                Some(Write::CommandFinished(done)) => finished(done, max_chars),
                Some(Write::Error(error)) => shell_error(error),
                None => "The input was sent.".to_owned(),
            }
        }
        ToolResult::ReadShellCommandOutput(result) => {
            use api::read_shell_command_output_result::Result as ReadOutput;
            match &result.result {
                Some(ReadOutput::LongRunningCommandSnapshot(running)) => {
                    snapshot(running, max_chars)
                }
                Some(ReadOutput::CommandFinished(done)) => finished(done, max_chars),
                Some(ReadOutput::Error(error)) => shell_error(error),
                None => "No output was available.".to_owned(),
            }
        }
        ToolResult::TransferShellCommandControlToUser(result) => {
            use api::transfer_shell_command_control_to_user_result::Result as Transfer;
            match &result.result {
                Some(Transfer::LongRunningCommandSnapshot(running)) => snapshot(running, max_chars),
                Some(Transfer::CommandFinished(done)) => finished(done, max_chars),
                Some(Transfer::Error(error)) => shell_error(error),
                None => "Control was handed to the user.".to_owned(),
            }
        }
        ToolResult::AskUserQuestion(result) => match &result.result {
            Some(api::ask_user_question_result::Result::Success(success)) => {
                use api::ask_user_question_result::answer_item::Answer;
                let answers: Vec<String> = success
                    .answers
                    .iter()
                    .map(|item| match &item.answer {
                        Some(Answer::MultipleChoice(choice)) => {
                            let mut answer = choice.selected_options.join(", ");
                            if !choice.other_text.trim().is_empty() {
                                if !answer.is_empty() {
                                    answer.push_str("; ");
                                }
                                answer.push_str(choice.other_text.trim());
                            }
                            format!("{}: {answer}", item.question_id)
                        }
                        Some(Answer::Skipped(())) | None => {
                            format!("{}: (skipped)", item.question_id)
                        }
                    })
                    .collect();
                format!("The user answered:\n{}", answers.join("\n"))
            }
            Some(api::ask_user_question_result::Result::Error(error)) => {
                format!("The question could not be asked: {}", error.message)
            }
            None => "The user did not answer.".to_owned(),
        },
        ToolResult::Cancel(()) => "The user cancelled this tool call.".to_owned(),
        other => truncate_middle(&format!("{other:?}"), 2_000),
    }
}

fn failed_reads(failed: &[api::read_files_result::FailedRead]) -> Vec<String> {
    failed
        .iter()
        .map(|read| format!("Could not read {}: {}", read.path, read.message))
        .collect()
}

/// Renders a user turn: attached context first, then the query itself. Images become image
/// parts so vision models can see them.
pub(crate) fn user_query(query: &api::message::UserQuery, max_chars: usize) -> Vec<ContentPart> {
    let mut sections = Vec::new();
    let mut images = Vec::new();

    let mut attachments: Vec<_> = query.referenced_attachments.iter().collect();
    attachments.sort_by(|(left, _), (right, _)| left.cmp(right));
    for (key, attachment) in attachments {
        if let Some(text) = attachment_text(attachment, max_chars) {
            sections.push(format!("Attachment `{key}`:\n{text}"));
        }
    }

    if let Some(context) = &query.context {
        #[allow(deprecated)]
        for command in &context.executed_shell_commands {
            sections.push(executed_command(command, max_chars));
        }
        for selected in &context.selected_text {
            sections.push(format!(
                "Selected text:\n{}",
                fenced(&selected.text, max_chars)
            ));
        }
        for file in &context.files {
            if let Some(content) = &file.content {
                sections.push(format!(
                    "Attached file {}",
                    file_content(content, max_chars)
                ));
            }
        }
        for image in &context.images {
            let mime = if image.mime_type.is_empty() {
                "image/png"
            } else {
                image.mime_type.as_str()
            };
            images.push(ContentPart::ImageUrl {
                image_url: ImageUrl {
                    url: format!(
                        "data:{mime};base64,{}",
                        base64::engine::general_purpose::STANDARD.encode(&image.data)
                    ),
                },
            });
        }
    }

    sections.push(query.query.clone());
    let mut parts = vec![ContentPart::Text {
        text: sections.join("\n\n"),
    }];
    parts.extend(images);
    parts
}

fn executed_command(command: &api::ExecutedShellCommand, max_chars: usize) -> String {
    format!(
        "Terminal block `{}` (exit code {}):\n{}",
        command.command,
        command.exit_code,
        fenced(&command.output, max_chars)
    )
}

fn attachment_text(attachment: &api::Attachment, max_chars: usize) -> Option<String> {
    use api::attachment::Value;
    Some(match attachment.value.as_ref()? {
        Value::PlainText(text) => truncate_middle(text, max_chars),
        Value::ExecutedShellCommand(command) => executed_command(command, max_chars),
        Value::RunningShellCommand(running) => format!(
            "Running command `{}`. {}",
            running.command,
            running
                .snapshot
                .as_ref()
                .map(|running| snapshot(running, max_chars))
                .unwrap_or_default()
        ),
        Value::DriveObject(object) => {
            use api::drive_object::ObjectPayload;
            match object.object_payload.as_ref()? {
                ObjectPayload::Workflow(workflow) => format!(
                    "Workflow \"{}\" ({}):\n{}",
                    workflow.name,
                    workflow.description,
                    fenced(&workflow.command, max_chars)
                ),
                ObjectPayload::Notebook(notebook) => format!(
                    "Notebook \"{}\":\n{}",
                    notebook.title,
                    truncate_middle(&notebook.content, max_chars)
                ),
                ObjectPayload::GenericStringObject(object) => {
                    truncate_middle(&object.payload, max_chars)
                }
            }
        }
        #[allow(deprecated)]
        Value::DiffHunk(hunk) => format!(
            "Diff of {}:\n{}",
            hunk.file_path,
            fenced(&hunk.diff_content, max_chars)
        ),
        Value::DiffSet(diff_set) => diff_set
            .hunks
            .iter()
            .map(|hunk| {
                format!(
                    "Diff of {}:\n{}",
                    hunk.file_path,
                    fenced(&hunk.diff_content, max_chars)
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n"),
        Value::DocumentContent(document) => truncate_middle(&document.content, max_chars),
        Value::FilePathReference(reference) => format!("File: {}", reference.file_path),
    })
}

#[cfg(test)]
#[path = "render_tests.rs"]
mod tests;
