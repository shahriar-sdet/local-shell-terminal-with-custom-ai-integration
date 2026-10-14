//! The tools offered to the model, and translation of its calls into Warp tool calls that the
//! client executes (and asks the user to approve) exactly as it would for Warp's own agent.

use local_ai::ToolDefinition;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use warp_multi_agent_api as api;
use warp_multi_agent_api::message::tool_call::{self as tc, Tool};
use warp_multi_agent_api::message::update_todos::Operation as TodoOperation;

/// A tool the model can call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolKind {
    RunShellCommand,
    ReadFiles,
    Grep,
    FindFiles,
    EditFiles,
    CreateFile,
    DeleteFiles,
    WriteToRunningCommand,
    ReadRunningCommandOutput,
    AskUserQuestion,
    /// Handled inside the engine: it updates the todo list shown in the conversation and needs
    /// no client round trip.
    UpdateTodos,
}

const ALL_TOOLS: [ToolKind; 11] = [
    ToolKind::RunShellCommand,
    ToolKind::ReadFiles,
    ToolKind::Grep,
    ToolKind::FindFiles,
    ToolKind::EditFiles,
    ToolKind::CreateFile,
    ToolKind::DeleteFiles,
    ToolKind::WriteToRunningCommand,
    ToolKind::ReadRunningCommandOutput,
    ToolKind::AskUserQuestion,
    ToolKind::UpdateTodos,
];

impl ToolKind {
    pub(crate) fn name(self) -> &'static str {
        match self {
            ToolKind::RunShellCommand => "run_shell_command",
            ToolKind::ReadFiles => "read_files",
            ToolKind::Grep => "grep",
            ToolKind::FindFiles => "find_files",
            ToolKind::EditFiles => "edit_files",
            ToolKind::CreateFile => "create_file",
            ToolKind::DeleteFiles => "delete_files",
            ToolKind::WriteToRunningCommand => "write_to_running_command",
            ToolKind::ReadRunningCommandOutput => "read_running_command_output",
            ToolKind::AskUserQuestion => "ask_user_question",
            ToolKind::UpdateTodos => "update_todos",
        }
    }

    pub(crate) fn from_name(name: &str) -> Option<Self> {
        let name = name.trim();
        ALL_TOOLS.into_iter().find(|tool| tool.name() == name)
    }

    /// The client capability the tool depends on, or `None` for engine-internal tools.
    fn required_tool_type(self) -> Option<api::ToolType> {
        match self {
            ToolKind::RunShellCommand => Some(api::ToolType::RunShellCommand),
            ToolKind::ReadFiles => Some(api::ToolType::ReadFiles),
            ToolKind::Grep => Some(api::ToolType::Grep),
            ToolKind::FindFiles => Some(api::ToolType::FileGlobV2),
            ToolKind::EditFiles | ToolKind::CreateFile | ToolKind::DeleteFiles => {
                Some(api::ToolType::ApplyFileDiffs)
            }
            ToolKind::WriteToRunningCommand => Some(api::ToolType::WriteToLongRunningShellCommand),
            ToolKind::ReadRunningCommandOutput => Some(api::ToolType::ReadShellCommandOutput),
            ToolKind::AskUserQuestion => Some(api::ToolType::AskUserQuestion),
            ToolKind::UpdateTodos => None,
        }
    }

    pub(crate) fn definition(self) -> ToolDefinition {
        let (description, parameters) = match self {
            ToolKind::RunShellCommand => (
                "Run a shell command in the user's current terminal session (same shell, \
                 working directory and environment). The user sees the command and may have to \
                 approve it. Avoid interactive programs and pagers: pass flags such as \
                 --no-pager or pipe to cat.",
                json!({
                    "type": "object",
                    "properties": {
                        "command": {
                            "type": "string",
                            "description": "The exact command line to run."
                        },
                        "risk": {
                            "type": "string",
                            "enum": [
                                "read_only",
                                "trivial_local_change",
                                "nontrivial_local_change",
                                "external_change",
                                "risky"
                            ],
                            "description": "read_only: changes nothing (ls, cat, git status). \
                                trivial_local_change: small, easily undone local change. \
                                nontrivial_local_change: meaningful local change (installs, \
                                builds that write files, git commit). external_change: affects \
                                anything outside this machine (git push, deploys, API calls). \
                                risky: destructive or hard to undo (rm -rf, force push, sudo)."
                        },
                        "wait_until_complete": {
                            "type": "boolean",
                            "description": "Defaults to true. Use false for processes that keep \
                                running (dev servers, watchers, REPLs); you then receive a \
                                command_id to use with read_running_command_output and \
                                write_to_running_command."
                        }
                    },
                    "required": ["command", "risk"]
                }),
            ),
            ToolKind::ReadFiles => (
                "Read one or more files. Optionally restrict each file to a 1-based inclusive \
                 line range; omit the range to read the whole file.",
                json!({
                    "type": "object",
                    "properties": {
                        "files": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "path": {"type": "string"},
                                    "start_line": {"type": "integer"},
                                    "end_line": {"type": "integer"}
                                },
                                "required": ["path"]
                            }
                        }
                    },
                    "required": ["files"]
                }),
            ),
            ToolKind::Grep => (
                "Search file contents for regular expressions. Returns the matching files and \
                 line numbers; use read_files to see the lines.",
                json!({
                    "type": "object",
                    "properties": {
                        "queries": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "Regular expressions to search for."
                        },
                        "path": {
                            "type": "string",
                            "description": "Directory or file to search. Defaults to the \
                                current directory."
                        }
                    },
                    "required": ["queries"]
                }),
            ),
            ToolKind::FindFiles => (
                "Find files by glob pattern (e.g. **/*.rs, src/**/test_*.py).",
                json!({
                    "type": "object",
                    "properties": {
                        "patterns": {"type": "array", "items": {"type": "string"}},
                        "search_dir": {
                            "type": "string",
                            "description": "Directory to search. Defaults to the current \
                                directory."
                        },
                        "max_matches": {"type": "integer"}
                    },
                    "required": ["patterns"]
                }),
            ),
            ToolKind::EditFiles => (
                "Edit existing files by exact search and replace. Each search string must \
                 match the current file content exactly once, including whitespace, so \
                 include a few surrounding lines. Read the file first.",
                json!({
                    "type": "object",
                    "properties": {
                        "summary": {
                            "type": "string",
                            "description": "One line describing the change."
                        },
                        "edits": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "file_path": {"type": "string"},
                                    "search": {"type": "string"},
                                    "replace": {"type": "string"}
                                },
                                "required": ["file_path", "search", "replace"]
                            }
                        }
                    },
                    "required": ["edits"]
                }),
            ),
            ToolKind::CreateFile => (
                "Create a file with the given content.",
                json!({
                    "type": "object",
                    "properties": {
                        "file_path": {"type": "string"},
                        "content": {"type": "string"},
                        "overwrite": {
                            "type": "boolean",
                            "description": "Replace the file if it already exists."
                        }
                    },
                    "required": ["file_path", "content"]
                }),
            ),
            ToolKind::DeleteFiles => (
                "Delete files.",
                json!({
                    "type": "object",
                    "properties": {
                        "file_paths": {"type": "array", "items": {"type": "string"}}
                    },
                    "required": ["file_paths"]
                }),
            ),
            ToolKind::WriteToRunningCommand => (
                "Send input to a command that is still running (e.g. answer a prompt, type \
                 into a REPL, or send Ctrl-C as \"\\u0003\" in raw mode).",
                json!({
                    "type": "object",
                    "properties": {
                        "command_id": {"type": "string"},
                        "input": {"type": "string"},
                        "mode": {
                            "type": "string",
                            "enum": ["line", "raw", "block"],
                            "description": "line: type the text and press Enter. raw: send \
                                bytes as-is. block: paste multi-line text."
                        }
                    },
                    "required": ["command_id", "input"]
                }),
            ),
            ToolKind::ReadRunningCommandOutput => (
                "Read the latest output of a command that is still running.",
                json!({
                    "type": "object",
                    "properties": {
                        "command_id": {"type": "string"},
                        "wait_seconds": {
                            "type": "number",
                            "description": "How long to wait before reading. Defaults to 5."
                        },
                        "until_complete": {
                            "type": "boolean",
                            "description": "Wait for the command to exit instead."
                        }
                    },
                    "required": ["command_id"]
                }),
            ),
            ToolKind::AskUserQuestion => (
                "Ask the user multiple-choice questions when a decision is genuinely theirs \
                 to make. They can also type their own answer.",
                json!({
                    "type": "object",
                    "properties": {
                        "questions": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "question": {"type": "string"},
                                    "options": {"type": "array", "items": {"type": "string"}},
                                    "multi_select": {"type": "boolean"},
                                    "recommended": {
                                        "type": "integer",
                                        "description": "Zero-based index of the option you \
                                            recommend."
                                    }
                                },
                                "required": ["question", "options"]
                            }
                        }
                    },
                    "required": ["questions"]
                }),
            ),
            ToolKind::UpdateTodos => (
                "Maintain a visible checklist for multi-step tasks. Send the complete list \
                 every time, marking finished items completed.",
                json!({
                    "type": "object",
                    "properties": {
                        "todos": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "id": {"type": "string"},
                                    "title": {"type": "string"},
                                    "status": {"type": "string", "enum": ["pending", "completed"]}
                                },
                                "required": ["id", "title", "status"]
                            }
                        }
                    },
                    "required": ["todos"]
                }),
            ),
        };
        ToolDefinition::function(self.name(), description, parameters)
    }
}

/// Tools the client said it can execute, plus the engine-internal ones.
pub(crate) fn available_tools(supported_tool_types: &[i32]) -> Vec<ToolKind> {
    ALL_TOOLS
        .into_iter()
        .filter(|tool| {
            tool.required_tool_type()
                .is_none_or(|required| supported_tool_types.contains(&(required as i32)))
        })
        .collect()
}

/// The model's original call, stored in the tool-call message's `server_message_data`, which the
/// client round-trips untouched. Lets later turns replay the call exactly as the model made it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct RecordedCall {
    /// Set for engine-internal calls, whose messages have no `tool_call_id` of their own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    pub arguments: String,
}

#[derive(Serialize, Deserialize)]
struct RecordEnvelope {
    local_tool_call: RecordedCall,
}

impl RecordedCall {
    pub(crate) fn encode(&self) -> String {
        serde_json::to_string(&RecordEnvelope {
            local_tool_call: self.clone(),
        })
        .unwrap_or_default()
    }

    pub(crate) fn decode(data: &str) -> Option<Self> {
        serde_json::from_str::<RecordEnvelope>(data)
            .ok()
            .map(|envelope| envelope.local_tool_call)
    }
}

pub(crate) enum ParsedCall {
    Client(Box<Tool>),
    Todos(Vec<TodoOperation>),
}

/// Validates the model's arguments and builds the corresponding Warp tool call. Errors are
/// phrased for the model, which gets them back so it can correct the call.
pub(crate) fn parse_call(
    tool: ToolKind,
    arguments: &str,
    has_todo_list: bool,
) -> Result<ParsedCall, String> {
    let args = parse_arguments(arguments)?;
    let tool = match tool {
        ToolKind::RunShellCommand => {
            let command = required_string(&args, &["command", "cmd"])?;
            let risk = match string(&args, &["risk"]).as_deref() {
                Some("read_only") => api::RiskCategory::ReadOnly,
                Some("trivial_local_change") => api::RiskCategory::TrivialLocalChange,
                Some("external_change") => api::RiskCategory::ExternalChange,
                Some("risky") => api::RiskCategory::Risky,
                Some("nontrivial_local_change") | Some(_) | None => {
                    api::RiskCategory::NontrivialLocalChange
                }
            };
            Tool::RunShellCommand(tc::RunShellCommand {
                command,
                is_read_only: risk == api::RiskCategory::ReadOnly,
                is_risky: risk == api::RiskCategory::Risky,
                risk_category: risk as i32,
                wait_until_complete_value: boolean(&args, &["wait_until_complete"])
                    .map(tc::run_shell_command::WaitUntilCompleteValue::WaitUntilComplete),
                ..Default::default()
            })
        }
        ToolKind::ReadFiles => {
            let files = file_requests(&args);
            if files.is_empty() {
                return Err("`files` must list at least one file with a `path`.".to_owned());
            }
            Tool::ReadFiles(tc::ReadFiles { files })
        }
        ToolKind::Grep => {
            let queries = strings(&args, &["queries", "query", "pattern", "patterns"]);
            if queries.is_empty() {
                return Err("`queries` must contain at least one pattern.".to_owned());
            }
            Tool::Grep(tc::Grep {
                queries,
                path: string(&args, &["path", "directory", "dir"]).unwrap_or_else(|| ".".into()),
            })
        }
        ToolKind::FindFiles => {
            let patterns = strings(&args, &["patterns", "pattern", "glob"]);
            if patterns.is_empty() {
                return Err("`patterns` must contain at least one glob.".to_owned());
            }
            Tool::FileGlobV2(tc::FileGlobV2 {
                patterns,
                search_dir: string(&args, &["search_dir", "path", "directory"])
                    .unwrap_or_else(|| ".".into()),
                max_matches: integer(&args, &["max_matches"]).unwrap_or(0) as i32,
                ..Default::default()
            })
        }
        ToolKind::EditFiles => {
            let diffs = edits(&args)?;
            Tool::ApplyFileDiffs(tc::ApplyFileDiffs {
                summary: string(&args, &["summary", "description"]).unwrap_or_default(),
                diffs,
                ..Default::default()
            })
        }
        ToolKind::CreateFile => Tool::ApplyFileDiffs(tc::ApplyFileDiffs {
            summary: string(&args, &["summary"]).unwrap_or_default(),
            new_files: vec![tc::apply_file_diffs::NewFile {
                file_path: required_string(&args, &["file_path", "path"])?,
                content: string(&args, &["content", "contents", "text"]).unwrap_or_default(),
                allow_overwrite: boolean(&args, &["overwrite", "allow_overwrite"]).unwrap_or(false),
            }],
            ..Default::default()
        }),
        ToolKind::DeleteFiles => {
            let paths = strings(&args, &["file_paths", "paths", "file_path", "path"]);
            if paths.is_empty() {
                return Err("`file_paths` must list at least one file.".to_owned());
            }
            Tool::ApplyFileDiffs(tc::ApplyFileDiffs {
                deleted_files: paths
                    .into_iter()
                    .map(|file_path| tc::apply_file_diffs::DeleteFile { file_path })
                    .collect(),
                ..Default::default()
            })
        }
        ToolKind::WriteToRunningCommand => {
            use tc::write_to_long_running_shell_command::{Mode, mode};
            let mode = match string(&args, &["mode"]).as_deref() {
                Some("raw") => mode::Mode::Raw(()),
                Some("block") => mode::Mode::Block(()),
                Some(_) | None => mode::Mode::Line(()),
            };
            Tool::WriteToLongRunningShellCommand(tc::WriteToLongRunningShellCommand {
                input: required_string(&args, &["input", "text"])?.into_bytes(),
                mode: Some(Mode { mode: Some(mode) }),
                command_id: required_string(&args, &["command_id"])?,
            })
        }
        ToolKind::ReadRunningCommandOutput => {
            use tc::read_shell_command_output::Delay;
            let delay = if boolean(&args, &["until_complete"]).unwrap_or(false) {
                Delay::OnCompletion(())
            } else {
                let seconds = number(&args, &["wait_seconds", "seconds"])
                    .unwrap_or(5.0)
                    .clamp(0.0, 600.0);
                Delay::Duration(prost_types::Duration {
                    seconds: seconds.trunc() as i64,
                    nanos: (seconds.fract() * 1e9) as i32,
                })
            };
            Tool::ReadShellCommandOutput(tc::ReadShellCommandOutput {
                command_id: required_string(&args, &["command_id"])?,
                delay: Some(delay),
            })
        }
        ToolKind::AskUserQuestion => Tool::AskUserQuestion(ask_user_question(&args)?),
        ToolKind::UpdateTodos => {
            return Ok(ParsedCall::Todos(todo_operations(&args, has_todo_list)?));
        }
    };
    Ok(ParsedCall::Client(Box::new(tool)))
}

/// Describes a Warp tool call the engine did not create (no recorded call), so history can still
/// show the model what happened.
pub(crate) fn describe_tool(tool: &Tool) -> RecordedCall {
    let (name, arguments) = match tool {
        Tool::RunShellCommand(call) => ("run_shell_command", json!({"command": call.command})),
        Tool::ReadFiles(call) => (
            "read_files",
            json!({"files": call.files.iter().map(|file| json!({"path": file.name})).collect::<Vec<_>>()}),
        ),
        Tool::Grep(call) => ("grep", json!({"queries": call.queries, "path": call.path})),
        Tool::FileGlobV2(call) => (
            "find_files",
            json!({"patterns": call.patterns, "search_dir": call.search_dir}),
        ),
        Tool::ApplyFileDiffs(call) => (
            "edit_files",
            json!({
                "summary": call.summary,
                "files": call.diffs.iter().map(|diff| diff.file_path.clone())
                    .chain(call.new_files.iter().map(|file| file.file_path.clone()))
                    .chain(call.deleted_files.iter().map(|file| file.file_path.clone()))
                    .collect::<Vec<_>>(),
            }),
        ),
        Tool::WriteToLongRunningShellCommand(call) => (
            "write_to_running_command",
            json!({"command_id": call.command_id, "input": String::from_utf8_lossy(&call.input)}),
        ),
        Tool::ReadShellCommandOutput(call) => (
            "read_running_command_output",
            json!({"command_id": call.command_id}),
        ),
        Tool::AskUserQuestion(call) => (
            "ask_user_question",
            json!({"questions": call.questions.iter().map(|question| question.question.clone()).collect::<Vec<_>>()}),
        ),
        other => (
            "warp_tool",
            json!({"tool": format!("{other:?}").chars().take(200).collect::<String>()}),
        ),
    };
    RecordedCall {
        id: None,
        name: name.to_owned(),
        arguments: arguments.to_string(),
    }
}

fn parse_arguments(arguments: &str) -> Result<Map<String, Value>, String> {
    let trimmed = arguments.trim();
    if trimmed.is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str::<Value>(trimmed) {
        Ok(Value::Object(map)) => Ok(map),
        // Some models double-encode the arguments object as a JSON string.
        Ok(Value::String(inner)) if inner.trim_start().starts_with('{') => parse_arguments(&inner),
        Ok(_) => Err("Tool arguments must be a JSON object.".to_owned()),
        Err(error) => Err(format!("Tool arguments are not valid JSON: {error}")),
    }
}

fn lookup<'a>(args: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .find_map(|key| args.get(*key))
        .filter(|value| !value.is_null())
}

fn string(args: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    match lookup(args, keys)? {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn required_string(args: &Map<String, Value>, keys: &[&str]) -> Result<String, String> {
    string(args, keys)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("Missing required argument `{}`.", keys[0]))
}

fn strings(args: &Map<String, Value>, keys: &[&str]) -> Vec<String> {
    match lookup(args, keys) {
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(|value| match value {
                Value::String(value) => Some(value.clone()),
                Value::Object(object) => object
                    .get("path")
                    .or_else(|| object.get("file_path"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                _ => None,
            })
            .filter(|value| !value.trim().is_empty())
            .collect(),
        Some(Value::String(value)) if !value.trim().is_empty() => vec![value.clone()],
        _ => Vec::new(),
    }
}

fn boolean(args: &Map<String, Value>, keys: &[&str]) -> Option<bool> {
    match lookup(args, keys)? {
        Value::Bool(value) => Some(*value),
        Value::String(value) => value.trim().parse().ok(),
        _ => None,
    }
}

fn number(args: &Map<String, Value>, keys: &[&str]) -> Option<f64> {
    match lookup(args, keys)? {
        Value::Number(value) => value.as_f64(),
        Value::String(value) => value.trim().parse().ok(),
        _ => None,
    }
}

fn integer(args: &Map<String, Value>, keys: &[&str]) -> Option<u32> {
    number(args, keys)
        .filter(|value| *value >= 0.0)
        .map(|value| value as u32)
}

fn file_requests(args: &Map<String, Value>) -> Vec<tc::read_files::File> {
    let entries: Vec<Map<String, Value>> =
        match lookup(args, &["files", "paths", "path", "file_path"]) {
            Some(Value::Array(values)) => values
                .iter()
                .filter_map(|value| match value {
                    Value::Object(object) => Some(object.clone()),
                    Value::String(path) => {
                        let mut object = Map::new();
                        object.insert("path".to_owned(), Value::String(path.clone()));
                        Some(object)
                    }
                    _ => None,
                })
                .collect(),
            Some(Value::String(_)) => vec![args.clone()],
            _ => Vec::new(),
        };
    entries
        .iter()
        .filter_map(|entry| {
            let path = string(entry, &["path", "file_path", "name"])?;
            let start = integer(entry, &["start_line", "start"]);
            let end = integer(entry, &["end_line", "end"]);
            let line_ranges = match (start, end) {
                (None, None) => Vec::new(),
                (start, end) => {
                    let start = start.unwrap_or(1).max(1);
                    let end = end.unwrap_or(u32::MAX).max(start);
                    vec![api::FileContentLineRange { start, end }]
                }
            };
            Some(tc::read_files::File {
                name: path,
                line_ranges,
            })
        })
        .collect()
}

fn edits(args: &Map<String, Value>) -> Result<Vec<tc::apply_file_diffs::FileDiff>, String> {
    let entries: Vec<Map<String, Value>> = match lookup(args, &["edits", "diffs", "changes"]) {
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(|value| value.as_object().cloned())
            .collect(),
        _ => vec![args.clone()],
    };
    let mut diffs = Vec::new();
    for entry in &entries {
        let file_path = required_string(entry, &["file_path", "path"])?;
        let search = string(entry, &["search", "old_string", "old", "find"])
            .ok_or_else(|| format!("Edit for {file_path} is missing `search`."))?;
        if search.is_empty() {
            return Err(format!(
                "Edit for {file_path} has an empty `search`; use create_file for new files."
            ));
        }
        let replace = string(entry, &["replace", "new_string", "new", "replacement"])
            .ok_or_else(|| format!("Edit for {file_path} is missing `replace`."))?;
        diffs.push(tc::apply_file_diffs::FileDiff {
            file_path,
            search,
            replace,
        });
    }
    if diffs.is_empty() {
        return Err("`edits` must contain at least one edit.".to_owned());
    }
    Ok(diffs)
}

fn ask_user_question(args: &Map<String, Value>) -> Result<api::AskUserQuestion, String> {
    use api::ask_user_question::{MultipleChoice, Option as ChoiceOption, Question, question};
    let Some(Value::Array(entries)) = lookup(args, &["questions"]) else {
        return Err("`questions` must be a list.".to_owned());
    };
    let mut questions = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let Some(entry) = entry.as_object() else {
            continue;
        };
        let text = required_string(entry, &["question", "text"])?;
        let options = strings(entry, &["options", "choices"]);
        if options.is_empty() {
            return Err(format!("Question \"{text}\" needs at least one option."));
        }
        let recommended = integer(entry, &["recommended", "recommended_option_index"])
            .filter(|index| (*index as usize) < options.len())
            .map_or(-1, |index| index as i32);
        questions.push(Question {
            question_id: format!("q{}", index + 1),
            question: text,
            question_type: Some(question::QuestionType::MultipleChoice(MultipleChoice {
                options: options
                    .into_iter()
                    .map(|label| ChoiceOption { label })
                    .collect(),
                recommended_option_index: recommended,
                is_multiselect: boolean(entry, &["multi_select", "multiselect"]).unwrap_or(false),
                supports_other: true,
            })),
        });
    }
    if questions.is_empty() {
        return Err("`questions` must contain at least one question.".to_owned());
    }
    Ok(api::AskUserQuestion { questions })
}

fn todo_operations(
    args: &Map<String, Value>,
    has_todo_list: bool,
) -> Result<Vec<TodoOperation>, String> {
    let Some(Value::Array(entries)) = lookup(args, &["todos", "items"]) else {
        return Err("`todos` must be a list.".to_owned());
    };
    let mut items = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let Some(entry) = entry.as_object() else {
            continue;
        };
        let title = required_string(entry, &["title", "content", "text"])?;
        let id = string(entry, &["id"]).unwrap_or_else(|| format!("todo-{}", index + 1));
        let is_completed = matches!(
            string(entry, &["status"]).as_deref(),
            Some("completed" | "done" | "complete")
        );
        let item = api::TodoItem {
            id,
            title,
            description: String::new(),
        };
        items.push((item, is_completed));
    }
    let completed_ids: Vec<String> = items
        .iter()
        .filter(|(_, is_completed)| *is_completed)
        .map(|(item, _)| item.id.clone())
        .collect();
    let mark_completed = (!completed_ids.is_empty()).then_some(TodoOperation::MarkTodosCompleted(
        api::MarkTodosCompleted {
            todo_ids: completed_ids,
        },
    ));
    let mut operations = Vec::new();
    if has_todo_list {
        // Completion must precede the pending update, which replaces the pending items and
        // would otherwise drop the items being completed.
        operations.extend(mark_completed);
        operations.push(TodoOperation::UpdatePendingTodos(api::UpdatePendingTodos {
            updated_pending_todos: items
                .into_iter()
                .filter(|(_, is_completed)| !is_completed)
                .map(|(item, _)| item)
                .collect(),
        }));
    } else {
        operations.push(TodoOperation::CreateTodoList(api::CreateTodoList {
            initial_todos: items.into_iter().map(|(item, _)| item).collect(),
        }));
        operations.extend(mark_completed);
    }
    Ok(operations)
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
