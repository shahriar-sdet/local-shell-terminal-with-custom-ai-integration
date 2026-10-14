use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use async_compat::CompatExt as _;
use bytes::Bytes;
use futures::stream::BoxStream;
use futures::{Stream, StreamExt as _};
use instant::Instant;
use url::Url;

use crate::config::{LocalAiConfig, ProviderKind, ReasoningEffort, TaskKind};
use crate::error::LocalAiError;
use crate::lm_studio;
use crate::stream::{Piece, SseDecoder, StreamAssembler, ThinkSplitter};
use crate::types::{
    ChatChunk, ChatCompletion, ChatMessage, ChatOutput, ChatRequest, FinishReason, ModelList,
    StreamEvent, StreamOptions, ToolDefinition,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const METADATA_TIMEOUT: Duration = Duration::from_secs(20);
/// How long a successful LM Studio model preparation is trusted before the load state is checked
/// again. Keeps rapid-fire requests (suggestions while typing) from polling the model list.
const PREPARED_MODEL_TTL: Duration = Duration::from_secs(60);
/// Servers report rejected optional parameters one at a time.
const MAX_PARAMETER_RETRIES: usize = 4;

pub type ChatStream = BoxStream<'static, Result<StreamEvent, LocalAiError>>;

/// A model offered by the provider.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelInfo {
    pub id: String,
    pub display_name: Option<String>,
    pub is_embedding: bool,
    pub max_context_length: Option<u32>,
    /// Context lengths of the instances LM Studio currently has loaded.
    pub loaded_context_lengths: Vec<u32>,
    pub supports_tools: Option<bool>,
    pub supports_vision: Option<bool>,
    /// LM Studio reasoning settings the model accepts (e.g. `off`, `low`, `high`).
    pub reasoning_options: Vec<String>,
}

/// Client for the user's OpenAI-compatible endpoint. Every request goes through
/// [`http_client::Client`], so the process-wide egress policy applies.
pub struct LocalAiClient {
    pub(crate) http: http_client::Client,
    provider: ProviderKind,
    base_url: Url,
    api_key: Option<String>,
    prepared_models: Mutex<HashMap<(String, u32), Instant>>,
    prepare_lock: futures::lock::Mutex<()>,
}

impl LocalAiClient {
    pub fn new(config: &LocalAiConfig) -> Result<Self, LocalAiError> {
        let base_url = config
            .parsed_base_url()
            .ok_or_else(|| LocalAiError::InvalidUrl(config.base_url.clone()))?;
        let http = http_client::Client::from_client_builder(
            reqwest::Client::builder().connect_timeout(CONNECT_TIMEOUT),
        )
        .map_err(|error| LocalAiError::from_transport(&error))?;
        let api_key = Some(config.api_key.trim().to_owned()).filter(|key| !key.is_empty());
        Ok(Self {
            http,
            provider: config.provider,
            base_url,
            api_key,
            prepared_models: Mutex::new(HashMap::new()),
            prepare_lock: futures::lock::Mutex::new(()),
        })
    }

    pub fn provider(&self) -> ProviderKind {
        self.provider
    }

    /// Builds a request for `task` using the generation settings the user chose for it.
    pub fn request_for_task(
        &self,
        config: &LocalAiConfig,
        task: TaskKind,
        messages: Vec<ChatMessage>,
        tools: Vec<ToolDefinition>,
    ) -> ChatRequest {
        let profile = config.profile(task);
        let uses_completion_tokens = self.base_url.host_str() == Some("api.openai.com");
        let reasoning_effort = match profile.reasoning {
            ReasoningEffort::ModelDefault => None,
            ReasoningEffort::Off => Some("none"),
            ReasoningEffort::Low => Some("low"),
            ReasoningEffort::Medium => Some("medium"),
            ReasoningEffort::High => Some("high"),
        };
        ChatRequest {
            model: config.model_for(task).to_owned(),
            messages,
            tools,
            temperature: profile.temperature,
            max_tokens: profile
                .max_output_tokens
                .filter(|_| !uses_completion_tokens),
            max_completion_tokens: profile.max_output_tokens.filter(|_| uses_completion_tokens),
            reasoning_effort: reasoning_effort.map(str::to_owned),
            stream: false,
            stream_options: None,
        }
    }

    /// Lists the models the endpoint offers. For LM Studio this includes load state and
    /// capabilities; other servers only report identifiers.
    pub async fn list_models(&self) -> Result<Vec<ModelInfo>, LocalAiError> {
        if self.provider == ProviderKind::LmStudio {
            match lm_studio::list_models(self).await {
                Ok(models) => return Ok(models.iter().map(lm_studio::Model::to_info).collect()),
                // Older LM Studio versions only expose the OpenAI-compatible listing.
                Err(error) => log::info!("LM Studio model listing unavailable: {error}"),
            }
        }
        let response = self.get(self.api_url("models")?).await?;
        let list: ModelList = parse_json(&read_text(response).await?)?;
        Ok(list
            .data
            .into_iter()
            .map(|entry| ModelInfo {
                id: entry.id,
                ..Default::default()
            })
            .collect())
    }

    /// Makes sure LM Studio has `model` loaded with at least `context_length` tokens of context,
    /// reloading it when the loaded instance is smaller. A no-op for other providers.
    pub async fn prepare_model(
        &self,
        model: &str,
        context_length: u32,
    ) -> Result<(), LocalAiError> {
        if self.provider != ProviderKind::LmStudio || model.is_empty() || context_length == 0 {
            return Ok(());
        }
        let key = (model.to_owned(), context_length);
        let _guard = self.prepare_lock.lock().await;
        if self
            .prepared_models
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&key)
            .is_some_and(|prepared_at| prepared_at.elapsed() < PREPARED_MODEL_TTL)
        {
            return Ok(());
        }
        lm_studio::ensure_loaded(self, model, context_length).await?;
        self.prepared_models
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(key, Instant::now());
        Ok(())
    }

    /// Sends a request and waits for the whole answer.
    pub async fn chat(&self, mut request: ChatRequest) -> Result<ChatOutput, LocalAiError> {
        request.stream = false;
        request.stream_options = None;
        let response = self.post_chat(request).await?;
        let completion: ChatCompletion = parse_json(&read_text(response).await?)?;
        let choice =
            completion.choices.into_iter().next().ok_or_else(|| {
                LocalAiError::Protocol("the response contained no choices".into())
            })?;
        let mut reasoning = choice
            .message
            .reasoning
            .or(choice.message.reasoning_content)
            .unwrap_or_default();
        let mut text = String::new();
        let mut splitter = ThinkSplitter::default();
        let content = choice.message.content.unwrap_or_default();
        for piece in splitter.push(&content).into_iter().chain(splitter.finish()) {
            match piece {
                Piece::Text(chunk) => text.push_str(&chunk),
                Piece::Reasoning(chunk) => reasoning.push_str(&chunk),
            }
        }
        let tool_calls = choice.message.tool_calls;
        let finish_reason = match choice.finish_reason.as_deref().map(FinishReason::parse) {
            Some(FinishReason::Stop) | None if !tool_calls.is_empty() => FinishReason::ToolCalls,
            Some(reason) => reason,
            None => FinishReason::Stop,
        };
        Ok(ChatOutput {
            text,
            reasoning,
            tool_calls,
            finish_reason,
            usage: completion.usage,
        })
    }

    /// Sends a request and streams the answer as it is generated.
    pub async fn chat_stream(&self, mut request: ChatRequest) -> Result<ChatStream, LocalAiError> {
        request.stream = true;
        request.stream_options = Some(StreamOptions {
            include_usage: true,
        });
        let response = self.post_chat(request).await?;
        Ok(decode_stream(response.bytes_stream()).boxed())
    }

    pub(crate) fn api_url(&self, path: &str) -> Result<Url, LocalAiError> {
        self.base_url
            .join(path)
            .map_err(|error| LocalAiError::InvalidUrl(error.to_string()))
    }

    /// The server root, i.e. the base URL without its trailing `/v1/` segment. LM Studio's
    /// native API lives under `/api/v1` on the same origin.
    pub(crate) fn root_url(&self, path: &str) -> Result<Url, LocalAiError> {
        let mut root = self.base_url.clone();
        let trimmed = root
            .path()
            .trim_end_matches('/')
            .trim_end_matches("/v1")
            .to_owned();
        root.set_path(&format!("{trimmed}/"));
        root.join(path)
            .map_err(|error| LocalAiError::InvalidUrl(error.to_string()))
    }

    pub(crate) async fn get(&self, url: Url) -> Result<http_client::Response, LocalAiError> {
        let builder = self.authorize(self.http.get(url)).timeout(METADATA_TIMEOUT);
        check_status(builder.send().await).await
    }

    pub(crate) async fn post_json(
        &self,
        url: Url,
        body: &impl serde::Serialize,
        timeout: Option<Duration>,
    ) -> Result<http_client::Response, LocalAiError> {
        let mut builder = self.authorize(self.http.post(url)).json(body);
        if let Some(timeout) = timeout {
            builder = builder.timeout(timeout);
        }
        check_status(builder.send().await).await
    }

    fn authorize<'a>(
        &self,
        builder: http_client::RequestBuilder<'a>,
    ) -> http_client::RequestBuilder<'a> {
        match &self.api_key {
            Some(key) => builder.bearer_auth(key),
            None => builder,
        }
    }

    /// Posts a chat completion, retrying without optional parameters the server rejects so a
    /// single preset works across LM Studio versions, OpenAI reasoning models and other servers.
    async fn post_chat(
        &self,
        mut request: ChatRequest,
    ) -> Result<http_client::Response, LocalAiError> {
        let url = self.api_url("chat/completions")?;
        let mut attempts = 0;
        loop {
            match self.post_json(url.clone(), &request, None).await {
                Err(LocalAiError::Http {
                    status: 400 | 422,
                    message,
                }) if attempts < MAX_PARAMETER_RETRIES
                    && drop_rejected_parameter(&mut request, &message) =>
                {
                    attempts += 1;
                    log::info!("AI endpoint rejected an optional parameter; retrying: {message}");
                }
                result => return result,
            }
        }
    }
}

/// Removes or swaps the optional parameter named in a 400 error. Returns `false` when the error
/// is about something else.
fn drop_rejected_parameter(request: &mut ChatRequest, message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    if request.reasoning_effort.is_some() && message.contains("reasoning") {
        request.reasoning_effort = None;
    } else if request.temperature.is_some() && message.contains("temperature") {
        request.temperature = None;
    } else if request.max_tokens.is_some() && message.contains("max_tokens") {
        request.max_completion_tokens = request.max_tokens.take();
    } else if request.max_completion_tokens.is_some() && message.contains("max_completion_tokens") {
        request.max_completion_tokens = None;
    } else if request.stream_options.is_some() && message.contains("stream_options") {
        request.stream_options = None;
    } else {
        return false;
    }
    true
}

async fn check_status(
    response: Result<http_client::Response, reqwest::Error>,
) -> Result<http_client::Response, LocalAiError> {
    let response = response.map_err(|error| LocalAiError::from_transport(&error))?;
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.text().await.unwrap_or_default();
    Err(LocalAiError::from_status(status.as_u16(), &body))
}

pub(crate) async fn read_text(response: http_client::Response) -> Result<String, LocalAiError> {
    response
        .text()
        .await
        .map_err(|error| LocalAiError::from_transport(&error))
}

pub(crate) fn parse_json<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, LocalAiError> {
    serde_json::from_str(text).map_err(|error| LocalAiError::Protocol(error.to_string()))
}

enum Payload {
    Done,
    Chunk(ChatChunk),
    Error(LocalAiError),
}

fn parse_payload(payload: &str) -> Option<Payload> {
    let payload = payload.trim();
    if payload.is_empty() {
        return None;
    }
    if payload == "[DONE]" {
        return Some(Payload::Done);
    }
    Some(match serde_json::from_str::<ChatChunk>(payload) {
        Ok(ChatChunk {
            error: Some(error), ..
        }) => Payload::Error(LocalAiError::from_status(500, &error.to_string())),
        Ok(chunk) => Payload::Chunk(chunk),
        Err(error) => Payload::Error(LocalAiError::Protocol(format!(
            "could not parse streamed chunk: {error}"
        ))),
    })
}

fn decode_stream(
    bytes: impl Stream<Item = reqwest::Result<Bytes>> + Send + 'static,
) -> impl Stream<Item = Result<StreamEvent, LocalAiError>> + Send + 'static {
    async_stream::stream! {
        let mut bytes = Box::pin(bytes);
        let mut decoder = SseDecoder::default();
        let mut assembler = StreamAssembler::default();
        let mut saw_done = false;
        // reqwest needs a Tokio context to poll its body; the app's executor isn't Tokio.
        'read: while let Some(chunk) = bytes.next().compat().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(error) => {
                    yield Err(LocalAiError::from_transport(&error));
                    return;
                }
            };
            for payload in decoder.push(&chunk) {
                match parse_payload(&payload) {
                    Some(Payload::Done) => {
                        saw_done = true;
                        break 'read;
                    }
                    Some(Payload::Chunk(chunk)) => {
                        for event in assembler.push_chunk(chunk) {
                            yield Ok(event);
                        }
                    }
                    Some(Payload::Error(error)) => {
                        yield Err(error);
                        return;
                    }
                    None => {}
                }
            }
        }
        if !saw_done
            && let Some(payload) = decoder.finish()
        {
            match parse_payload(&payload) {
                Some(Payload::Done) => saw_done = true,
                Some(Payload::Chunk(chunk)) => {
                    for event in assembler.push_chunk(chunk) {
                        yield Ok(event);
                    }
                }
                Some(Payload::Error(error)) => {
                    yield Err(error);
                    return;
                }
                None => {}
            }
        }
        let (events, output) = assembler.finish(saw_done);
        for event in events {
            yield Ok(event);
        }
        yield Ok(StreamEvent::Finished(output));
    }
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
