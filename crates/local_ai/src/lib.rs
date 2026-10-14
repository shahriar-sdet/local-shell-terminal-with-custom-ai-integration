//! Client for the AI endpoint a LocalShell user configures: LM Studio or any server speaking
//! OpenAI's chat completions API. Nothing in this crate knows about Warp's own servers.

mod client;
mod config;
mod error;
mod lm_studio;
mod stream;
mod types;

pub use client::{ChatStream, LocalAiClient, ModelInfo};
pub use config::{LocalAiConfig, ProviderKind, ReasoningEffort, TaskKind, TaskProfile};
pub use error::LocalAiError;
pub use types::{
    ChatMessage, ChatOutput, ChatRequest, ContentPart, FinishReason, FunctionCall,
    FunctionDefinition, ImageUrl, StreamEvent, ToolCall, ToolDefinition, Usage, UserContent,
};
