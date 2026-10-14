//! Incremental decoding of streamed chat completions.

use std::collections::BTreeMap;

use crate::types::{ChatChunk, ChatOutput, FinishReason, StreamEvent, ToolCall, Usage};

const THINK_OPEN: &str = "<think>";
const THINK_CLOSE: &str = "</think>";

/// Splits a server-sent-events byte stream into the payloads of its `data:` lines.
#[derive(Default)]
pub(crate) struct SseDecoder {
    buffer: Vec<u8>,
}

impl SseDecoder {
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        self.buffer.extend_from_slice(bytes);
        let mut payloads = Vec::new();
        while let Some(newline) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=newline).collect();
            if let Some(payload) = data_payload(&line) {
                payloads.push(payload);
            }
        }
        payloads
    }

    /// Returns the payload of a trailing line that was not newline-terminated.
    pub(crate) fn finish(&mut self) -> Option<String> {
        let line = std::mem::take(&mut self.buffer);
        data_payload(&line)
    }
}

fn data_payload(line: &[u8]) -> Option<String> {
    let line = String::from_utf8_lossy(line);
    let line = line.trim_end_matches(['\n', '\r']);
    line.strip_prefix("data:")
        .map(|data| data.strip_prefix(' ').unwrap_or(data).to_owned())
}

#[derive(Debug, PartialEq)]
pub(crate) enum Piece {
    Text(String),
    Reasoning(String),
}

/// Separates `<think>...</think>` sections that some local models emit inline in their content
/// from the visible answer. Tags split across chunks are handled.
#[derive(Default)]
pub(crate) struct ThinkSplitter {
    in_think: bool,
    pending: String,
}

impl ThinkSplitter {
    pub(crate) fn push(&mut self, chunk: &str) -> Vec<Piece> {
        self.pending.push_str(chunk);
        let mut pieces = Vec::new();
        loop {
            let tag = if self.in_think {
                THINK_CLOSE
            } else {
                THINK_OPEN
            };
            if let Some(index) = self.pending.find(tag) {
                let before: String = self.pending.drain(..index).collect();
                self.emit(before, &mut pieces);
                self.pending.drain(..tag.len());
                self.in_think = !self.in_think;
                continue;
            }
            let keep = partial_tag_suffix_len(&self.pending, tag);
            let ready: String = self.pending.drain(..self.pending.len() - keep).collect();
            self.emit(ready, &mut pieces);
            return pieces;
        }
    }

    pub(crate) fn finish(&mut self) -> Vec<Piece> {
        let mut pieces = Vec::new();
        let rest = std::mem::take(&mut self.pending);
        self.emit(rest, &mut pieces);
        pieces
    }

    fn emit(&self, text: String, pieces: &mut Vec<Piece>) {
        if text.is_empty() {
            return;
        }
        pieces.push(if self.in_think {
            Piece::Reasoning(text)
        } else {
            Piece::Text(text)
        });
    }
}

/// Length of the longest proper prefix of `tag` that `text` ends with. Tags are ASCII, so the
/// result is always a char boundary.
fn partial_tag_suffix_len(text: &str, tag: &str) -> usize {
    (1..tag.len())
        .rev()
        .find(|len| text.ends_with(&tag[..*len]))
        .unwrap_or(0)
}

#[derive(Default)]
struct PartialToolCall {
    id: String,
    name: String,
    arguments: String,
}

/// Accumulates streamed chunks into user-visible deltas and a final [`ChatOutput`].
#[derive(Default)]
pub(crate) struct StreamAssembler {
    splitter: ThinkSplitter,
    text: String,
    reasoning: String,
    tool_calls: BTreeMap<usize, PartialToolCall>,
    finish_reason: Option<FinishReason>,
    usage: Option<Usage>,
}

impl StreamAssembler {
    pub(crate) fn push_chunk(&mut self, chunk: ChatChunk) -> Vec<StreamEvent> {
        if let Some(usage) = chunk.usage {
            self.usage = Some(usage);
        }
        let mut events = Vec::new();
        for choice in chunk.choices {
            let delta = choice.delta;
            if let Some(reasoning) = delta.reasoning.or(delta.reasoning_content)
                && !reasoning.is_empty()
            {
                self.push_piece(Piece::Reasoning(reasoning), &mut events);
            }
            if let Some(content) = delta.content {
                for piece in self.splitter.push(&content) {
                    self.push_piece(piece, &mut events);
                }
            }
            for call in delta.tool_calls {
                let partial = self.tool_calls.entry(call.index).or_default();
                if let Some(id) = call.id
                    && !id.is_empty()
                {
                    partial.id = id;
                }
                if let Some(function) = call.function {
                    if let Some(name) = function.name {
                        partial.name.push_str(&name);
                    }
                    if let Some(arguments) = function.arguments {
                        partial.arguments.push_str(&arguments);
                    }
                }
            }
            if let Some(reason) = choice.finish_reason {
                self.finish_reason = Some(FinishReason::parse(&reason));
            }
        }
        events
    }

    /// Flushes buffered text and assembles the final output. `saw_done` reports whether the
    /// server sent its end-of-stream marker; without it a missing finish reason means the
    /// stream was cut off.
    pub(crate) fn finish(mut self, saw_done: bool) -> (Vec<StreamEvent>, ChatOutput) {
        let mut events = Vec::new();
        for piece in self.splitter.finish() {
            self.push_piece(piece, &mut events);
        }
        let tool_calls: Vec<ToolCall> = std::mem::take(&mut self.tool_calls)
            .into_iter()
            .filter(|(_, call)| !call.name.is_empty())
            .map(|(index, call)| {
                let id = if call.id.is_empty() {
                    format!("call_{index}")
                } else {
                    call.id
                };
                ToolCall::new(id, call.name, call.arguments)
            })
            .collect();
        let finish_reason = match self.finish_reason.take() {
            // Some servers report "stop" even when the turn ends in tool calls.
            Some(FinishReason::Stop) | None if !tool_calls.is_empty() => FinishReason::ToolCalls,
            Some(reason) => reason,
            None if saw_done => FinishReason::Stop,
            None => FinishReason::Other("stream ended unexpectedly".to_owned()),
        };
        let output = ChatOutput {
            text: self.text,
            reasoning: self.reasoning,
            tool_calls,
            finish_reason,
            usage: self.usage,
        };
        (events, output)
    }

    fn push_piece(&mut self, piece: Piece, events: &mut Vec<StreamEvent>) {
        match piece {
            Piece::Text(text) => {
                self.text.push_str(&text);
                events.push(StreamEvent::Text(text));
            }
            Piece::Reasoning(reasoning) => {
                self.reasoning.push_str(&reasoning);
                events.push(StreamEvent::Reasoning(reasoning));
            }
        }
    }
}

#[cfg(test)]
#[path = "stream_tests.rs"]
mod tests;
