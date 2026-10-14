/// Failures talking to the user's AI endpoint, phrased for display to the user.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LocalAiError {
    #[error(
        "No AI provider is configured. Open Settings > AI to choose LM Studio or an \
         OpenAI-compatible endpoint and pick a model."
    )]
    NotConfigured,
    #[error("The AI endpoint URL is not a valid http(s) URL: {0}")]
    InvalidUrl(String),
    #[error("{0}")]
    Blocked(String),
    #[error("Could not reach the AI endpoint: {0}")]
    Connection(String),
    #[error("The AI endpoint rejected the API key (HTTP {status}).")]
    Unauthorized { status: u16 },
    #[error(
        "The request does not fit in the model's context window. Increase the context length \
         for this task in Settings > AI, or start a new conversation. ({0})"
    )]
    ContextLengthExceeded(String),
    #[error("The AI endpoint returned HTTP {status}: {message}")]
    Http { status: u16, message: String },
    #[error("Unexpected response from the AI endpoint: {0}")]
    Protocol(String),
}

impl LocalAiError {
    pub(crate) fn from_status(status: u16, body: &str) -> Self {
        let message = error_message_from_body(body);
        let lower = message.to_ascii_lowercase();
        if matches!(status, 401 | 403) {
            return LocalAiError::Unauthorized { status };
        }
        if is_context_length_error(&lower) {
            return LocalAiError::ContextLengthExceeded(message);
        }
        LocalAiError::Http { status, message }
    }

    pub(crate) fn from_transport(error: &(dyn std::error::Error + 'static)) -> Self {
        let mut source = Some(error);
        let mut chain = Vec::new();
        while let Some(current) = source {
            if let Some(blocked) = current.downcast_ref::<egress_guard::Blocked>() {
                return LocalAiError::Blocked(blocked.to_string());
            }
            chain.push(current.to_string());
            source = current.source();
        }
        chain.dedup();
        LocalAiError::Connection(chain.join(": "))
    }
}

/// Extracts the human-readable message from an OpenAI-style `{"error": {"message": ...}}` body,
/// falling back to the raw body.
fn error_message_from_body(body: &str) -> String {
    let parsed: Option<serde_json::Value> = serde_json::from_str(body).ok();
    let message = parsed.as_ref().and_then(|value| {
        let error = value.get("error").unwrap_or(value);
        error
            .get("message")
            .and_then(serde_json::Value::as_str)
            .or_else(|| error.as_str())
            .map(str::to_owned)
    });
    let message = message.unwrap_or_else(|| body.trim().to_owned());
    const MAX_LEN: usize = 500;
    if message.len() > MAX_LEN {
        let mut end = MAX_LEN;
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &message[..end])
    } else {
        message
    }
}

fn is_context_length_error(lower_message: &str) -> bool {
    [
        "context_length_exceeded",
        "context length",
        "context window",
        "maximum context",
        "n_ctx",
        "too many tokens",
        "prompt is too long",
    ]
    .iter()
    .any(|needle| lower_message.contains(needle))
}
