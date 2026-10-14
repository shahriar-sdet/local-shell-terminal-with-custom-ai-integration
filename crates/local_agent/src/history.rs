//! Reconstruction of the chat transcript from the conversation's task messages.

use local_ai::{ChatMessage, ContentPart, ToolCall, UserContent};
use warp_multi_agent_api as api;

use crate::render;
use crate::tools::{RecordedCall, describe_tool};

pub(crate) const TODOS_UPDATED: &str = "The todo list was updated.";
const NO_RESULT: &str = "This tool call returned no result; it was skipped or cancelled.";
const SUPERSEDED: &str = "This tool call was not run; the user sent a new message instead.";

/// Builds chat messages from task messages, oldest first.
///
/// OpenAI-style APIs reject a transcript in which an assistant tool call has no matching tool
/// message, so calls the user skipped or cancelled get a synthesized result.
pub(crate) fn build(messages: &[api::Message], max_tool_output_chars: usize) -> Vec<ChatMessage> {
    let mut builder = Builder {
        max_chars: max_tool_output_chars,
        ..Default::default()
    };
    for message in messages {
        builder.push(message);
    }
    builder.finish()
}

/// Whether the conversation already has a todo list, which decides between creating one and
/// updating it.
pub(crate) fn has_todo_list(messages: &[api::Message]) -> bool {
    messages
        .iter()
        .any(|message| matches!(message.message, Some(api::message::Message::UpdateTodos(_))))
}

#[derive(Default)]
struct Builder {
    max_chars: usize,
    output: Vec<ChatMessage>,
    assistant_text: String,
    assistant_calls: Vec<ToolCall>,
    /// Results the engine produced itself, emitted right after their assistant message.
    internal_results: Vec<(String, String)>,
    /// Client tool calls still waiting for a result.
    open_calls: Vec<String>,
}

impl Builder {
    fn push(&mut self, message: &api::Message) {
        use api::message::Message;
        let Some(kind) = &message.message else {
            return;
        };
        match kind {
            Message::UserQuery(query) => {
                self.flush_assistant();
                self.close_open_calls(SUPERSEDED);
                let parts = render::user_query(query, self.max_chars);
                self.output.push(ChatMessage::User {
                    content: user_content(parts),
                });
            }
            Message::AgentOutput(output) => {
                if !output.text.trim().is_empty() {
                    if !self.assistant_text.is_empty() {
                        self.assistant_text.push_str("\n\n");
                    }
                    self.assistant_text.push_str(output.text.trim());
                }
            }
            Message::ToolCall(call) => {
                let recorded = RecordedCall::decode(&message.server_message_data)
                    .or_else(|| call.tool.as_ref().map(describe_tool));
                if let Some(recorded) = recorded {
                    self.assistant_calls.push(ToolCall::new(
                        call.tool_call_id.clone(),
                        recorded.name,
                        recorded.arguments,
                    ));
                    self.open_calls.push(call.tool_call_id.clone());
                }
            }
            Message::UpdateTodos(_) => {
                // Only the first message of an update carries the model's call; the rest are
                // further operations of the same call.
                if let Some(recorded) = RecordedCall::decode(&message.server_message_data)
                    && let Some(id) = recorded.id.clone()
                {
                    self.assistant_calls.push(ToolCall::new(
                        id.clone(),
                        recorded.name,
                        recorded.arguments,
                    ));
                    self.internal_results.push((id, TODOS_UPDATED.to_owned()));
                }
            }
            Message::ToolCallResult(result) => {
                self.flush_assistant();
                let Some(position) = self
                    .open_calls
                    .iter()
                    .position(|id| *id == result.tool_call_id)
                else {
                    log::debug!("Dropping tool result without a matching call");
                    return;
                };
                self.open_calls.remove(position);
                self.output.push(ChatMessage::Tool {
                    tool_call_id: result.tool_call_id.clone(),
                    content: render::tool_result(result.result.as_ref(), self.max_chars),
                });
            }
            // Reasoning is not replayed: it is large and models don't need it to continue.
            // The remaining kinds have no meaning for a local model.
            _ => {}
        }
    }

    fn flush_assistant(&mut self) {
        if self.assistant_text.is_empty() && self.assistant_calls.is_empty() {
            return;
        }
        let text = std::mem::take(&mut self.assistant_text);
        self.output.push(ChatMessage::Assistant {
            content: (!text.is_empty()).then_some(text),
            tool_calls: std::mem::take(&mut self.assistant_calls),
        });
        for (tool_call_id, content) in std::mem::take(&mut self.internal_results) {
            self.output.push(ChatMessage::Tool {
                tool_call_id,
                content,
            });
        }
    }

    fn close_open_calls(&mut self, reason: &str) {
        for tool_call_id in std::mem::take(&mut self.open_calls) {
            self.output.push(ChatMessage::Tool {
                tool_call_id,
                content: reason.to_owned(),
            });
        }
    }

    fn finish(mut self) -> Vec<ChatMessage> {
        self.flush_assistant();
        self.close_open_calls(NO_RESULT);
        self.output
    }
}

fn user_content(parts: Vec<ContentPart>) -> UserContent {
    match parts.as_slice() {
        [ContentPart::Text { text }] => UserContent::Text(text.clone()),
        _ => UserContent::Parts(parts),
    }
}

/// Characters per token used to estimate prompt size. Conservative for code and logs, which
/// tokenize less efficiently than prose.
const CHARS_PER_TOKEN: usize = 3;
const TRIMMED_TOOL_RESULT: &str = "[Output omitted to fit the model's context window.]";

pub(crate) fn estimate_tokens(chars: usize) -> usize {
    chars.div_ceil(CHARS_PER_TOKEN)
}

/// Shrinks `history` until it fits in `budget_chars`, keeping it a valid transcript: whole older
/// turns are dropped first, then old tool outputs in the current turn are replaced by a stub.
/// The latest user message is always kept.
pub(crate) fn fit_to_budget(
    mut history: Vec<ChatMessage>,
    budget_chars: usize,
) -> Vec<ChatMessage> {
    let size = |history: &[ChatMessage]| -> usize {
        history.iter().map(ChatMessage::approximate_len).sum()
    };
    let user_turn_starts = |history: &[ChatMessage]| -> Vec<usize> {
        history
            .iter()
            .enumerate()
            .filter(|(_, message)| matches!(message, ChatMessage::User { .. }))
            .map(|(index, _)| index)
            .collect()
    };

    while size(&history) > budget_chars {
        let starts = user_turn_starts(&history);
        let cut = match starts.as_slice() {
            [first, ..] if *first > 0 => *first,
            [_, second, ..] => *second,
            _ => break,
        };
        history.drain(..cut);
    }

    let mut index = 0;
    while size(&history) > budget_chars && index < history.len() {
        if let ChatMessage::Tool { content, .. } = &mut history[index]
            && content.len() > TRIMMED_TOOL_RESULT.len()
        {
            *content = TRIMMED_TOOL_RESULT.to_owned();
        }
        index += 1;
    }
    history
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod tests;
