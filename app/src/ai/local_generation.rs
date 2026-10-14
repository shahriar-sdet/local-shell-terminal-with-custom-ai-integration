//! Single-shot AI generations (command search, explanations, workflow metadata, suggestions)
//! answered by the user's own model.

use local_ai::{ChatMessage, LocalAiError, TaskKind};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::ai::generate_code_review_content::api::{GenerateCodeReviewContentRequest, OutputType};
use crate::ai::local_ai_runtime;
use crate::ai::predict::generate_ai_input_suggestions::{
    GenerateAIInputSuggestionsRequest, GenerateAIInputSuggestionsResponseV2,
};
use crate::ai::predict::generate_am_query_suggestions::{
    GenerateAMQuerySuggestionsRequest, GenerateAMQuerySuggestionsResponse, SimpleQuery,
    Suggestion as AgentQuerySuggestion,
};
use crate::ai::predict::predict_am_queries::{PredictAMQueriesRequest, PredictAMQueriesResponse};
use crate::ai_assistant::execution_context::WarpAiExecutionContext;

/// Sends one prompt for `task` and returns the model's answer text.
pub(crate) async fn complete(
    task: TaskKind,
    system: &str,
    messages: Vec<ChatMessage>,
) -> Result<String, LocalAiError> {
    let runtime = local_ai_runtime::current_or_error()?;
    let model = runtime.config.model_for(task).to_owned();
    let context_length = runtime.config.profile(task).context_length;
    if runtime.config.manage_model_loading
        && let Err(error) = runtime.client.prepare_model(&model, context_length).await
    {
        log::warn!("Could not prepare model {model}: {error}");
    }
    let mut chat = vec![ChatMessage::system(system)];
    chat.extend(messages);
    let request = runtime
        .client
        .request_for_task(&runtime.config, task, chat, Vec::new());
    let output = runtime.client.chat(request).await?;
    Ok(output.text.trim().to_owned())
}

/// Like [`complete`], but parses the answer as JSON of type `T`.
pub(crate) async fn complete_json<T: DeserializeOwned>(
    task: TaskKind,
    system: &str,
    user: String,
) -> Result<T, LocalAiError> {
    let text = complete(task, system, vec![ChatMessage::user(user)]).await?;
    json_answer(&text)
}

/// Parses a model answer as JSON of type `T`, or reports what the model said instead.
fn json_answer<T: DeserializeOwned>(text: &str) -> Result<T, LocalAiError> {
    parse_json_answer(text).ok_or_else(|| {
        LocalAiError::Protocol(format!(
            "the model did not answer with the expected JSON: {}",
            text.chars().take(200).collect::<String>()
        ))
    })
}

/// Extracts a JSON value from a model answer. Small models often wrap JSON in prose or code
/// fences, so the outermost object or array in the text is tried when the whole text isn't JSON.
pub(crate) fn parse_json_answer<T: DeserializeOwned>(text: &str) -> Option<T> {
    let text = text.trim();
    if let Ok(value) = serde_json::from_str(text) {
        return Some(value);
    }
    [('{', '}'), ('[', ']')]
        .into_iter()
        .find_map(|(open, close)| {
            let start = text.find(open)?;
            let end = text.rfind(close)?;
            (end > start)
                .then(|| serde_json::from_str(&text[start..=end]).ok())
                .flatten()
        })
}

/// A one-line description of the user's shell environment for prompts.
pub(crate) fn environment_line(context: Option<&WarpAiExecutionContext>) -> String {
    let Some(context) = context else {
        return "The user is on macOS.".to_owned();
    };
    let os = [
        context.os.category.as_deref(),
        context.os.distribution.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ");
    let shell = match &context.shell_version {
        Some(version) => format!("{} {version}", context.shell_name),
        None => context.shell_name.clone(),
    };
    format!(
        "The user is on {} using the {shell} shell.",
        if os.is_empty() { "macOS" } else { os.as_str() }
    )
}

#[derive(Debug, Deserialize)]
pub(crate) struct GeneratedCommands {
    #[serde(default)]
    pub commands: Vec<GeneratedCommand>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GeneratedCommand {
    pub command: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub parameters: Vec<GeneratedParameter>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GeneratedParameter {
    #[serde(alias = "name")]
    pub id: String,
    #[serde(default)]
    pub description: String,
}

/// Example requests and answers sent ahead of the user's request. Small models follow worked
/// examples far more reliably than instructions alone: without them, questions such as "what is
/// the list command?" came back with no commands.
const COMMAND_GENERATION_EXAMPLES: &[(&str, &str)] = &[
    (
        "what is the copy command?",
        r#"{"commands":[{"command":"cp {{source}} {{destination}}","description":"Copy a file","parameters":[{"id":"source","description":"File to copy"},{"id":"destination","description":"Where to put the copy"}]},{"command":"cp -R {{source}} {{destination}}","description":"Copy a folder","parameters":[{"id":"source","description":"Folder to copy"},{"id":"destination","description":"Where to put the copy"}]}]}"#,
    ),
    ("write a poem about the sea", r#"{"commands":[]}"#),
];

pub(crate) async fn generate_commands(
    prompt: String,
    context: Option<WarpAiExecutionContext>,
) -> Result<GeneratedCommands, LocalAiError> {
    let system = format!(
        "You suggest shell commands. {} The user's text may be a task, a question about a command \
         or a few keywords: in every case, suggest the commands that do what they mean or answer \
         their question. Reply with JSON only, no prose: \
         {{\"commands\":[{{\"command\":\"...\",\"description\":\"...\",\"parameters\":\
         [{{\"id\":\"...\",\"description\":\"...\"}}]}}]}}. Give one to three alternatives, best \
         first. Write {{{{name}}}} placeholders for values the user must supply and describe each \
         in parameters. The description is a short title. Answer {{\"commands\":[]}} only when no \
         shell command could help at all.",
        environment_line(context.as_ref())
    );
    let mut messages = COMMAND_GENERATION_EXAMPLES
        .iter()
        .flat_map(|(request, answer)| {
            [ChatMessage::user(*request), ChatMessage::assistant(*answer)]
        })
        .collect::<Vec<_>>();
    messages.push(ChatMessage::user(prompt));
    let text = complete(TaskKind::CommandGeneration, &system, messages).await?;
    json_answer(&text)
}

#[derive(Debug, Deserialize)]
pub(crate) struct GeneratedMetadata {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub arguments: Vec<GeneratedMetadataArgument>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GeneratedMetadataArgument {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub default_value: String,
}

pub(crate) async fn generate_command_metadata(
    command: String,
) -> Result<GeneratedMetadata, LocalAiError> {
    const SYSTEM: &str = "You document shell commands as reusable workflows. Reply with JSON \
        only: {\"title\":\"...\",\"description\":\"...\",\"command\":\"...\",\"arguments\":\
        [{\"name\":\"...\",\"description\":\"...\",\"default_value\":\"...\"}]}. The title is a \
        few words, the description one sentence. In command, replace values a user would change \
        (paths, names, numbers) with {{argument_name}} placeholders and list each in arguments \
        with the original value as default_value. Keep everything else identical.";
    complete_json(TaskKind::Utility, SYSTEM, command).await
}

/// Longest tail of recent terminal output included in suggestion prompts. Suggestions run while
/// the user types, so their prompts stay small.
const MAX_SUGGESTION_CONTEXT_CHARS: usize = 6_000;

fn recent_context(messages: &[String]) -> String {
    let joined = messages.join("\n\n");
    if joined.len() <= MAX_SUGGESTION_CONTEXT_CHARS {
        return joined;
    }
    let mut start = joined.len() - MAX_SUGGESTION_CONTEXT_CHARS;
    while !joined.is_char_boundary(start) {
        start += 1;
    }
    joined[start..].to_owned()
}

#[derive(Debug, Deserialize)]
struct PredictedCommand {
    #[serde(default)]
    command: String,
}

/// Predicts the shell command the user is most likely to run next.
pub(crate) async fn next_command(
    request: &GenerateAIInputSuggestionsRequest,
) -> Result<GenerateAIInputSuggestionsResponseV2, LocalAiError> {
    const SYSTEM: &str = "Predict the single shell command the user is most likely to run next. \
        Reply with JSON only: {\"command\":\"...\"}. Reply {\"command\":\"\"} when nothing is \
        clearly likely.";
    let mut prompt = String::new();
    if let Some(system_context) = &request.system_context {
        prompt.push_str(&format!("Environment: {system_context}\n\n"));
    }
    prompt.push_str(&format!(
        "Recent terminal blocks:\n{}\n",
        recent_context(&request.context_messages)
    ));
    if !request.history_context.trim().is_empty() {
        prompt.push_str(&format!(
            "\nCommands the user ran in similar situations:\n{}\n",
            request.history_context
        ));
    }
    if !request.rejected_suggestions.is_empty() {
        prompt.push_str(&format!(
            "\nDo not suggest any of: {}\n",
            request.rejected_suggestions.join(", ")
        ));
    }
    if let Some(prefix) = &request.prefix {
        prompt.push_str(&format!(
            "\nThe user has already typed `{prefix}`; the command must start with exactly that.\n"
        ));
    }
    let predicted: PredictedCommand = complete_json(TaskKind::Suggestions, SYSTEM, prompt).await?;
    let command = predicted.command.trim().to_owned();
    let matches_prefix = request
        .prefix
        .as_deref()
        .is_none_or(|prefix| command.starts_with(prefix));
    let commands = if command.is_empty() || !matches_prefix {
        Vec::new()
    } else {
        vec![command]
    };
    Ok(GenerateAIInputSuggestionsResponseV2 {
        most_likely_action: commands.first().cloned().unwrap_or_default(),
        commands,
        ai_queries: Vec::new(),
    })
}

#[derive(Debug, Deserialize)]
struct SuggestedQuery {
    #[serde(default)]
    query: String,
}

/// Suggests a request for the agent after a command finishes, typically to fix an error.
pub(crate) async fn agent_query_suggestion(
    request: &GenerateAMQuerySuggestionsRequest,
) -> Result<GenerateAMQuerySuggestionsResponse, LocalAiError> {
    const SYSTEM: &str = "A shell command just finished in the user's terminal. If an AI coding \
        agent could clearly help with what happened (usually an error), suggest one short \
        request the user could send it, such as \"Fix the failing test in parser.rs\". Reply \
        with JSON only: {\"query\":\"...\"}, or {\"query\":\"\"} when no help is needed.";
    let mut prompt = String::new();
    if let Some(system_context) = &request.system_context {
        prompt.push_str(&format!("Environment: {system_context}\n\n"));
    }
    prompt.push_str(&format!(
        "Exit code: {}\nTerminal blocks:\n{}",
        request.exit_code,
        recent_context(&request.context_messages)
    ));
    let suggested: SuggestedQuery = complete_json(TaskKind::Suggestions, SYSTEM, prompt).await?;
    let query = suggested.query.trim().to_owned();
    Ok(GenerateAMQuerySuggestionsResponse {
        id: uuid::Uuid::new_v4().to_string(),
        suggestion: (!query.is_empty()).then_some(AgentQuerySuggestion::Simple(SimpleQuery {
            query,
            should_plan_task: false,
        })),
    })
}

#[derive(Debug, Deserialize)]
struct CompletedQuery {
    #[serde(default)]
    query: String,
}

/// Completes a partially typed agent request. Returns only the text to append.
pub(crate) async fn complete_agent_query(
    request: &PredictAMQueriesRequest,
) -> Result<PredictAMQueriesResponse, LocalAiError> {
    const SYSTEM: &str = "Complete the user's partially typed request to an AI coding agent in \
        their terminal. Reply with JSON only: {\"query\":\"<the full request>\"}. The full \
        request must start with exactly the text already typed.";
    let mut prompt = String::new();
    if let Some(system_context) = &request.system_context {
        prompt.push_str(&format!("Environment: {system_context}\n\n"));
    }
    prompt.push_str(&format!(
        "Recent terminal blocks:\n{}\n\nTyped so far: {}",
        recent_context(&request.context_messages),
        request.partial_query
    ));
    let completed: CompletedQuery = complete_json(TaskKind::Suggestions, SYSTEM, prompt).await?;
    let suffix = completed
        .query
        .strip_prefix(&request.partial_query)
        .filter(|suffix| !suffix.trim().is_empty())
        .ok_or_else(|| LocalAiError::Protocol("no usable completion".to_owned()))?;
    Ok(PredictAMQueriesResponse {
        suggestion: suffix.to_owned(),
    })
}

/// Writes a commit message, pull request title or pull request description for a diff.
pub(crate) async fn code_review_content(
    request: &GenerateCodeReviewContentRequest,
) -> Result<String, LocalAiError> {
    let instruction = match request.output_type {
        OutputType::CommitMessage => {
            "Write a git commit message for this diff: an imperative subject line of at most 72 \
             characters, then a blank line and a short body explaining what changed and why, if \
             that adds anything. Reply with the commit message only."
        }
        OutputType::PrTitle => {
            "Write a pull request title of at most 72 characters for these changes. Reply with \
             the title only."
        }
        OutputType::PrDescription => {
            "Write a pull request description in Markdown for these changes: a short summary, \
             then a bullet list of the notable changes. Reply with the description only."
        }
    };
    let context_length = local_ai_runtime::current()
        .map(|runtime| runtime.config.profile(TaskKind::Utility).context_length)
        .filter(|tokens| *tokens > 0)
        .unwrap_or(4_096);
    // Leave room for the instruction and the answer; diffs tokenize at about 3 characters each.
    let max_diff_chars = context_length as usize * 3 * 6 / 10;
    let mut prompt = String::new();
    if !request.branch_name.is_empty() {
        prompt.push_str(&format!("Branch: {}\n", request.branch_name));
    }
    if !request.commit_messages.is_empty() {
        prompt.push_str(&format!(
            "Commits:\n{}\n",
            request.commit_messages.join("\n")
        ));
    }
    let mut diff_end = request.diff.len().min(max_diff_chars);
    while !request.diff.is_char_boundary(diff_end) {
        diff_end -= 1;
    }
    prompt.push_str(&format!("Diff:\n{}", &request.diff[..diff_end]));
    if diff_end < request.diff.len() {
        prompt.push_str("\n[diff truncated]");
    }
    let text = complete(
        TaskKind::Utility,
        instruction,
        vec![ChatMessage::user(prompt)],
    )
    .await?;
    Ok(strip_code_fence(&text).to_owned())
}

/// Removes a Markdown code fence wrapped around a whole answer.
fn strip_code_fence(text: &str) -> &str {
    let trimmed = text.trim();
    let Some(inner) = trimmed.strip_prefix("```") else {
        return trimmed;
    };
    let inner = inner.split_once('\n').map_or("", |(_, rest)| rest);
    inner.strip_suffix("```").unwrap_or(inner).trim()
}

#[cfg(test)]
#[path = "local_generation_tests.rs"]
mod tests;
