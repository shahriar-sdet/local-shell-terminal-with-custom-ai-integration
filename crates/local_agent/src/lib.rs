//! Warp's multi-agent protocol served inside the client, backed by the user's own model.
//!
//! Warp's agent normally runs on Warp's servers: the client sends a [`api::Request`] and applies
//! the streamed [`api::ResponseEvent`]s, executing the tool calls they contain. This crate
//! produces the same event stream from an OpenAI-compatible endpoint, so every agent feature of
//! the client (tool approval, file diffs, long-running commands, todo lists) keeps working while
//! nothing leaves the machine except requests to the configured endpoint.

mod convert;
mod events;
mod history;
mod prompt;
mod render;
mod tools;

use std::sync::Arc;

use futures::stream::BoxStream;
use futures::{Stream, StreamExt as _};
use instant::Instant;
use local_ai::{
    ChatMessage, FinishReason, LocalAiClient, LocalAiConfig, LocalAiError, StreamEvent, TaskKind,
    ToolCall, ToolDefinition,
};
use uuid::Uuid;
use warp_multi_agent_api as api;
use warp_multi_agent_api::client_action::Action;
use warp_multi_agent_api::response_event::stream_finished;

use crate::events::MessageFactory;
use crate::tools::{ParsedCall, RecordedCall, ToolKind};

/// Model calls per request that may happen without handing a tool call to the client (todo
/// updates, retries after malformed calls). Bounds runaway loops on weak models.
const MAX_MODEL_STEPS: usize = 6;
/// Assumed context window when the user leaves the context length at "server default".
const DEFAULT_CONTEXT_TOKENS: u32 = 32_768;

const INIT_PROJECT_PROMPT: &str = "Explore this project: its structure, languages, how to \
build, test and lint it, and its conventions. Then create an AGENTS.md file at the repository \
root that tells AI agents how to work in this codebase: a short overview, the exact build, test \
and lint commands, code style rules, and known pitfalls. Keep it concise.";

const UNSUPPORTED_INPUT: &str =
    "This action relies on Warp's cloud services, which are not part of LocalShell.";

/// Runs one agent request against the user's model.
///
/// `config` must already name the model to use for [`TaskKind::Agent`].
pub fn generate_multi_agent_output(
    client: Arc<LocalAiClient>,
    config: LocalAiConfig,
    request: api::Request,
) -> BoxStream<'static, api::ResponseEvent> {
    run(client, config, request).boxed()
}

enum InputKind {
    Agent,
    PassiveSuggestions,
    Unsupported,
}

struct PreparedInput {
    echoed: Vec<api::Message>,
    plan_mode: bool,
    kind: InputKind,
}

fn run(
    client: Arc<LocalAiClient>,
    config: LocalAiConfig,
    request: api::Request,
) -> impl Stream<Item = api::ResponseEvent> + Send + 'static {
    async_stream::stream! {
        let conversation_id = request
            .metadata
            .as_ref()
            .map(|metadata| metadata.conversation_id.clone())
            .filter(|id| !id.is_empty())
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let request_id = Uuid::new_v4().to_string();
        yield events::init(&conversation_id, &request_id);

        let tasks = request
            .task_context
            .map(|context| context.tasks)
            .unwrap_or_default();
        let root_task = tasks.into_iter().find(|task| {
            task.dependencies
                .as_ref()
                .is_none_or(|dependencies| dependencies.parent_task_id.is_empty())
        });
        let is_new_task = root_task.is_none();
        let (task_id, mut task_messages) = match root_task {
            Some(task) => (task.id, task.messages),
            None => (Uuid::new_v4().to_string(), Vec::new()),
        };
        let factory = MessageFactory {
            task_id: task_id.clone(),
            request_id,
        };
        let supported_tools = request
            .settings
            .as_ref()
            .map(|settings| settings.supported_tools.clone())
            .unwrap_or_default();

        let Some(input) = request.input else {
            yield events::finished(events::done(), None);
            return;
        };
        let context = input.context.clone();
        let PreparedInput { echoed, plan_mode, kind } = prepare_input(input, &factory);

        let mut setup = Vec::new();
        if is_new_task {
            setup.push(events::create_root_task(&task_id));
        }
        match kind {
            InputKind::Agent => {}
            InputKind::PassiveSuggestions => {
                // Proactive suggestions are produced by the dedicated suggestions task, not by
                // the agent conversation.
                yield events::finished(events::done(), None);
                return;
            }
            InputKind::Unsupported => {
                setup.push(events::add_messages(
                    &task_id,
                    vec![agent_output(&factory, UNSUPPORTED_INPUT)],
                ));
                yield events::actions(setup);
                yield events::finished(events::done(), None);
                return;
            }
        }
        if !echoed.is_empty() {
            setup.push(events::add_messages(&task_id, echoed.clone()));
        }
        if !setup.is_empty() {
            yield events::actions(setup);
        }
        task_messages.extend(echoed);

        let available_tools = tools::available_tools(&supported_tools);
        let tool_definitions: Vec<ToolDefinition> =
            available_tools.iter().map(|tool| tool.definition()).collect();
        let profile = config.profile(TaskKind::Agent).clone();
        let model = config.model_for(TaskKind::Agent).to_owned();
        let context_tokens = if profile.context_length > 0 {
            profile.context_length
        } else {
            DEFAULT_CONTEXT_TOKENS
        };
        let max_tool_output_chars = (context_tokens as usize / 2).clamp(2_000, 40_000);
        let system_prompt = prompt::system_prompt(context.as_ref(), &available_tools, plan_mode);
        let mut transcript = history::build(&task_messages, max_tool_output_chars);
        let mut has_todo_list = history::has_todo_list(&task_messages);

        if config.manage_model_loading
            && let Err(error) = client.prepare_model(&model, profile.context_length).await
        {
            // The chat request below reports a clearer error if the model really can't load.
            log::warn!("Could not prepare model {model}: {error}");
        }

        let budget_chars = prompt_budget_chars(
            context_tokens,
            profile.max_output_tokens,
            &system_prompt,
            &tool_definitions,
        );
        let mut usage = None;
        for step in 0..MAX_MODEL_STEPS {
            let mut messages = vec![ChatMessage::system(system_prompt.clone())];
            messages.extend(history::fit_to_budget(transcript.clone(), budget_chars));
            let chat_request = client.request_for_task(
                &config,
                TaskKind::Agent,
                messages,
                tool_definitions.clone(),
            );
            let mut stream = match client.chat_stream(chat_request).await {
                Ok(stream) => stream,
                Err(error) => {
                    yield events::finished(error_reason(error), None);
                    return;
                }
            };

            let mut streamer = OutputStreamer::new(&factory);
            let mut output = None;
            while let Some(event) = stream.next().await {
                match event {
                    Ok(StreamEvent::Reasoning(delta)) => {
                        yield events::actions(streamer.reasoning(delta));
                    }
                    Ok(StreamEvent::Text(delta)) => {
                        let actions = streamer.text(&delta);
                        if !actions.is_empty() {
                            yield events::actions(actions);
                        }
                    }
                    Ok(StreamEvent::Finished(finished)) => {
                        output = Some(finished);
                        break;
                    }
                    Err(error) => {
                        let actions = streamer.finish_reasoning();
                        if !actions.is_empty() {
                            yield events::actions(actions);
                        }
                        yield events::finished(error_reason(error), None);
                        return;
                    }
                }
            }
            let actions = streamer.finish_reasoning();
            if !actions.is_empty() {
                yield events::actions(actions);
            }
            let Some(output) = output else {
                yield events::finished(
                    events::internal_error("The AI endpoint closed the stream early."),
                    None,
                );
                return;
            };
            usage = output.usage.or(usage);

            if output.tool_calls.is_empty() {
                let note = match output.finish_reason {
                    FinishReason::Length => Some(
                        "\n\n_Stopped at the agent's maximum output length (Settings > AI)._",
                    ),
                    FinishReason::ContentFilter => Some("\n\n_The model's output was filtered._"),
                    _ if !streamer.wrote_text() => Some("The model returned an empty response."),
                    _ => None,
                };
                if let Some(note) = note {
                    yield events::actions(streamer.text(note));
                }
                break;
            }

            let outcome = handle_tool_calls(&output.tool_calls, &available_tools, has_todo_list, &factory);
            has_todo_list |= outcome.updated_todos;
            if !outcome.messages.is_empty() {
                yield events::actions(vec![events::add_messages(&task_id, outcome.messages)]);
            }
            if outcome.has_client_calls {
                // The client runs the calls (after approval) and sends the results in a new
                // request, which continues the conversation.
                break;
            }
            transcript.push(ChatMessage::Assistant {
                content: Some(output.text.trim().to_owned()).filter(|text| !text.is_empty()),
                tool_calls: output.tool_calls.clone(),
            });
            transcript.extend(outcome.feedback.into_iter().map(|(tool_call_id, content)| {
                ChatMessage::Tool {
                    tool_call_id,
                    content,
                }
            }));
            if step + 1 == MAX_MODEL_STEPS {
                yield events::actions(vec![events::add_messages(
                    &task_id,
                    vec![agent_output(
                        &factory,
                        "_Stopped: the model kept making calls that could not be run._",
                    )],
                )]);
            }
        }

        let usage_metadata = usage.map(|usage| stream_finished::ConversationUsageMetadata {
            context_window_usage: (usage.prompt_tokens as f32 / context_tokens as f32).min(1.0),
            total_input_tokens: usage.prompt_tokens,
            ..Default::default()
        });
        yield events::finished(events::done(), usage_metadata);
    }
}

#[allow(deprecated)]
fn prepare_input(input: api::request::Input, factory: &MessageFactory) -> PreparedInput {
    use api::request::input::Type;
    use api::request::input::user_inputs::user_input::Input as UserInput;

    let context = input.context;
    let mut prepared = PreparedInput {
        echoed: Vec::new(),
        plan_mode: false,
        kind: InputKind::Agent,
    };
    let push_query = |prepared: &mut PreparedInput, query: api::request::input::UserQuery| {
        prepared.plan_mode |= query
            .mode
            .as_ref()
            .is_some_and(|mode| matches!(mode.r#type, Some(api::user_query_mode::Type::Plan(()))));
        prepared
            .echoed
            .push(factory.new_message(convert::user_query_message(query, context.clone())));
    };
    let text =
        |text: String| factory.new_message(convert::text_query_message(text, context.clone()));

    match input.r#type {
        Some(Type::UserInputs(inputs)) => {
            for user_input in inputs.inputs {
                match user_input.input {
                    Some(UserInput::UserQuery(query)) => push_query(&mut prepared, query),
                    Some(UserInput::CliAgentUserQuery(query)) => {
                        if let Some(query) = query.user_query {
                            push_query(&mut prepared, query);
                        }
                    }
                    Some(UserInput::ToolCallResult(result)) => {
                        prepared.echoed.push(factory.new_message(
                            convert::tool_call_result_message(result, context.clone()),
                        ))
                    }
                    Some(
                        UserInput::MessagesReceivedFromAgents(_)
                        | UserInput::EventsFromAgents(_)
                        | UserInput::PassiveSuggestionResult(_)
                        | UserInput::OrchestrationConfigUpdate(_)
                        | UserInput::ConversationHandoff(_)
                        | UserInput::AgentWake(_),
                    )
                    | None => {}
                }
            }
        }
        Some(Type::UserQuery(query)) => push_query(&mut prepared, query),
        Some(Type::ToolCallResult(result)) => prepared
            .echoed
            .push(factory.new_message(convert::tool_call_result_message(result, context.clone()))),
        Some(Type::QueryWithCannedResponse(query)) => prepared.echoed.push(text(query.query)),
        Some(Type::AutoCodeDiffQuery(query)) => prepared.echoed.push(text(query.query)),
        Some(Type::ResumeConversation(_)) => prepared
            .echoed
            .push(text("Continue where you left off.".to_owned())),
        Some(Type::InitProjectRules(_)) => {
            prepared.echoed.push(text(INIT_PROJECT_PROMPT.to_owned()))
        }
        Some(Type::CreateNewProject(project)) => prepared
            .echoed
            .push(text(format!("Create a new project: {}", project.query))),
        Some(Type::CloneRepository(repository)) => prepared.echoed.push(text(format!(
            "Clone the git repository {} into the current directory, then summarize how to \
             build and run it.",
            repository.url
        ))),
        Some(Type::SummarizeConversation(summary)) => {
            let mut prompt = "Summarize this conversation so far: the goal, what was done, \
                              the current state, and open next steps."
                .to_owned();
            if !summary.prompt.trim().is_empty() {
                prompt.push_str("\n\n");
                prompt.push_str(summary.prompt.trim());
            }
            prepared.echoed.push(text(prompt));
        }
        Some(Type::InvokeSkill(invocation)) => {
            let mut prompt = String::new();
            if let Some(skill) = &invocation.skill {
                let name = skill
                    .descriptor
                    .as_ref()
                    .map(|descriptor| descriptor.name.as_str())
                    .unwrap_or("skill");
                prompt.push_str(&format!("Follow the \"{name}\" skill:\n\n"));
                if let Some(content) = &skill.content {
                    prompt.push_str(&content.content);
                }
            }
            if let Some(query) = &invocation.user_query
                && !query.query.trim().is_empty()
            {
                prompt.push_str("\n\nTask: ");
                prompt.push_str(&query.query);
            }
            prepared.echoed.push(text(prompt));
        }
        Some(Type::GeneratePassiveSuggestions(_)) => prepared.kind = InputKind::PassiveSuggestions,
        Some(
            Type::CodeReview(_)
            | Type::FetchReviewComments(_)
            | Type::CreateEnvironment(_)
            | Type::StartFromAmbientRunPrompt(_),
        ) => prepared.kind = InputKind::Unsupported,
        None => {}
    }
    prepared
}

struct ToolCallOutcome {
    messages: Vec<api::Message>,
    /// Results for calls the engine resolved itself, fed back to the model.
    feedback: Vec<(String, String)>,
    has_client_calls: bool,
    updated_todos: bool,
}

fn handle_tool_calls(
    calls: &[ToolCall],
    available: &[ToolKind],
    has_todo_list: bool,
    factory: &MessageFactory,
) -> ToolCallOutcome {
    let mut outcome = ToolCallOutcome {
        messages: Vec::new(),
        feedback: Vec::new(),
        has_client_calls: false,
        updated_todos: false,
    };
    let mut client_messages = Vec::new();
    for call in calls {
        let record = RecordedCall {
            id: None,
            name: call.function.name.clone(),
            arguments: call.function.arguments.clone(),
        };
        let Some(kind) =
            ToolKind::from_name(&call.function.name).filter(|kind| available.contains(kind))
        else {
            let names: Vec<&str> = available.iter().map(|tool| tool.name()).collect();
            outcome.feedback.push((
                call.id.clone(),
                format!(
                    "Unknown tool `{}`. Available tools: {}.",
                    call.function.name,
                    names.join(", ")
                ),
            ));
            continue;
        };
        match tools::parse_call(
            kind,
            &call.function.arguments,
            has_todo_list || outcome.updated_todos,
        ) {
            Ok(ParsedCall::Client(tool)) => {
                let mut message =
                    factory.new_message(api::message::Message::ToolCall(api::message::ToolCall {
                        tool_call_id: call.id.clone(),
                        tool: Some(*tool),
                    }));
                message.server_message_data = record.encode();
                client_messages.push(message);
                outcome.has_client_calls = true;
            }
            Ok(ParsedCall::Todos(operations)) => {
                for (index, operation) in operations.into_iter().enumerate() {
                    let mut message = factory.new_message(api::message::Message::UpdateTodos(
                        api::message::UpdateTodos {
                            operation: Some(operation),
                        },
                    ));
                    if index == 0 {
                        message.server_message_data = RecordedCall {
                            id: Some(call.id.clone()),
                            ..record.clone()
                        }
                        .encode();
                    }
                    outcome.messages.push(message);
                }
                outcome.updated_todos = true;
                outcome
                    .feedback
                    .push((call.id.clone(), history::TODOS_UPDATED.to_owned()));
            }
            Err(error) => outcome.feedback.push((
                call.id.clone(),
                format!("The call was not run: {error} Fix the arguments and try again."),
            )),
        }
    }
    outcome.messages.extend(client_messages);
    outcome
}

/// Streams the model's reasoning and answer into agent messages, creating each message on its
/// first delta and appending to it afterwards.
struct OutputStreamer<'a> {
    factory: &'a MessageFactory,
    reasoning: Option<OpenReasoning>,
    text_message_id: Option<String>,
}

struct OpenReasoning {
    message_id: String,
    started_at: Instant,
    text: String,
}

impl<'a> OutputStreamer<'a> {
    fn new(factory: &'a MessageFactory) -> Self {
        Self {
            factory,
            reasoning: None,
            text_message_id: None,
        }
    }

    fn wrote_text(&self) -> bool {
        self.text_message_id.is_some()
    }

    fn reasoning(&mut self, delta: String) -> Vec<Action> {
        let task_id = &self.factory.task_id;
        if let Some(open) = &mut self.reasoning {
            open.text.push_str(&delta);
            let message = self.factory.message_with_id(
                open.message_id.clone(),
                api::message::Message::AgentReasoning(api::message::AgentReasoning {
                    reasoning: delta,
                    finished_duration: None,
                }),
            );
            return vec![events::append(
                task_id,
                message,
                events::AGENT_REASONING_TEXT,
            )];
        }
        let message = self
            .factory
            .new_message(api::message::Message::AgentReasoning(
                api::message::AgentReasoning {
                    reasoning: delta.clone(),
                    finished_duration: None,
                },
            ));
        self.reasoning = Some(OpenReasoning {
            message_id: message.id.clone(),
            started_at: Instant::now(),
            text: delta,
        });
        vec![events::add_messages(task_id, vec![message])]
    }

    fn text(&mut self, delta: &str) -> Vec<Action> {
        let mut actions = self.finish_reasoning();
        let task_id = &self.factory.task_id;
        match &self.text_message_id {
            Some(message_id) => {
                let message = self.factory.message_with_id(
                    message_id.clone(),
                    api::message::Message::AgentOutput(api::message::AgentOutput {
                        text: delta.to_owned(),
                    }),
                );
                actions.push(events::append(task_id, message, events::AGENT_OUTPUT_TEXT));
            }
            None => {
                let delta = delta.trim_start();
                if delta.is_empty() {
                    return actions;
                }
                let message = agent_output(self.factory, delta);
                self.text_message_id = Some(message.id.clone());
                actions.push(events::add_messages(task_id, vec![message]));
            }
        }
        actions
    }

    /// Marks open reasoning as finished, which collapses it in the UI.
    fn finish_reasoning(&mut self) -> Vec<Action> {
        let Some(open) = self.reasoning.take() else {
            return Vec::new();
        };
        let elapsed = open.started_at.elapsed();
        let message = self.factory.message_with_id(
            open.message_id,
            api::message::Message::AgentReasoning(api::message::AgentReasoning {
                reasoning: open.text,
                finished_duration: Some(prost_types::Duration {
                    seconds: elapsed.as_secs() as i64,
                    nanos: elapsed.subsec_nanos() as i32,
                }),
            }),
        );
        vec![events::update(
            &self.factory.task_id,
            message,
            events::AGENT_REASONING_DURATION,
        )]
    }
}

fn agent_output(factory: &MessageFactory, text: &str) -> api::Message {
    factory.new_message(api::message::Message::AgentOutput(
        api::message::AgentOutput {
            text: text.to_owned(),
        },
    ))
}

/// Characters of conversation history that fit next to the system prompt, the tool definitions
/// and the space reserved for the answer.
fn prompt_budget_chars(
    context_tokens: u32,
    max_output_tokens: Option<u32>,
    system_prompt: &str,
    tools: &[ToolDefinition],
) -> usize {
    let context_tokens = context_tokens as usize;
    let reserved_output = max_output_tokens
        .map(|tokens| tokens as usize)
        .unwrap_or(context_tokens / 4)
        .clamp(512, context_tokens / 2);
    let fixed_chars = system_prompt.len()
        + serde_json::to_string(tools).map_or(0, |definitions| definitions.len());
    let available_tokens = context_tokens
        .saturating_sub(reserved_output)
        .saturating_sub(history::estimate_tokens(fixed_chars));
    // Leave headroom for per-message overhead and estimation error.
    available_tokens * 3 * 9 / 10
}

fn error_reason(error: LocalAiError) -> stream_finished::Reason {
    match error {
        LocalAiError::ContextLengthExceeded(_) => stream_finished::Reason::ContextWindowExceeded(
            stream_finished::ContextWindowExceeded {},
        ),
        other => events::internal_error(other.to_string()),
    }
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
