//! LM Studio's native REST API (`/api/v1`), used for model load state and loading models with a
//! specific context length. Chat itself goes through the OpenAI-compatible endpoint, because the
//! native chat endpoint has no tool calling.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::client::{LocalAiClient, ModelInfo, parse_json, read_text};
use crate::error::LocalAiError;

/// Loading a large model from disk can take minutes on a laptop.
const LOAD_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    #[serde(default)]
    models: Vec<Model>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Model {
    #[serde(rename = "type", default)]
    kind: String,
    key: String,
    display_name: Option<String>,
    max_context_length: Option<u32>,
    #[serde(default)]
    loaded_instances: Vec<LoadedInstance>,
    capabilities: Option<Capabilities>,
}

impl Model {
    pub(crate) fn to_info(&self) -> ModelInfo {
        ModelInfo {
            id: self.key.clone(),
            display_name: self.display_name.clone(),
            is_embedding: self.kind == "embedding",
            max_context_length: self.max_context_length,
            loaded_context_lengths: self
                .loaded_instances
                .iter()
                .map(|instance| instance.config.context_length)
                .collect(),
            supports_tools: self
                .capabilities
                .as_ref()
                .map(|capabilities| capabilities.trained_for_tool_use),
            supports_vision: self
                .capabilities
                .as_ref()
                .map(|capabilities| capabilities.vision),
            reasoning_options: self
                .capabilities
                .as_ref()
                .and_then(|capabilities| capabilities.reasoning.as_ref())
                .map(|reasoning| reasoning.allowed_options.clone())
                .unwrap_or_default(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct LoadedInstance {
    id: String,
    config: InstanceConfig,
}

#[derive(Debug, Deserialize)]
struct InstanceConfig {
    #[serde(default)]
    context_length: u32,
}

#[derive(Debug, Deserialize)]
struct Capabilities {
    #[serde(default)]
    vision: bool,
    #[serde(default)]
    trained_for_tool_use: bool,
    reasoning: Option<ReasoningCapability>,
}

#[derive(Debug, Deserialize)]
struct ReasoningCapability {
    #[serde(default)]
    allowed_options: Vec<String>,
}

#[derive(Serialize)]
struct LoadRequest<'a> {
    model: &'a str,
    context_length: u32,
}

#[derive(Serialize)]
struct UnloadRequest<'a> {
    instance_id: &'a str,
}

pub(crate) async fn list_models(client: &LocalAiClient) -> Result<Vec<Model>, LocalAiError> {
    let response = client.get(client.root_url("api/v1/models")?).await?;
    let models: ModelsResponse = parse_json(&read_text(response).await?)?;
    Ok(models.models)
}

/// Loads `model` with at least `context_length` tokens of context (capped at the model's maximum).
/// An instance already loaded with enough context is reused; smaller instances are unloaded first
/// so two copies of the same weights never compete for memory.
pub(crate) async fn ensure_loaded(
    client: &LocalAiClient,
    model: &str,
    context_length: u32,
) -> Result<(), LocalAiError> {
    let models = list_models(client).await?;
    let Some(entry) = models.iter().find(|candidate| candidate.key == model) else {
        // Not a key LM Studio knows (e.g. a custom identifier); its just-in-time loading applies.
        log::info!("LM Studio has no model with key {model}; skipping explicit load");
        return Ok(());
    };
    let wanted = entry
        .max_context_length
        .map_or(context_length, |max| context_length.min(max));
    if entry
        .loaded_instances
        .iter()
        .any(|instance| instance.config.context_length >= wanted)
    {
        return Ok(());
    }
    for instance in &entry.loaded_instances {
        log::info!(
            "Unloading {} (context {}) to reload with context {wanted}",
            instance.id,
            instance.config.context_length
        );
        client
            .post_json(
                client.root_url("api/v1/models/unload")?,
                &UnloadRequest {
                    instance_id: &instance.id,
                },
                Some(LOAD_TIMEOUT),
            )
            .await?;
    }
    log::info!("Loading {model} in LM Studio with context {wanted}");
    client
        .post_json(
            client.root_url("api/v1/models/load")?,
            &LoadRequest {
                model,
                context_length: wanted,
            },
            Some(LOAD_TIMEOUT),
        )
        .await?;
    Ok(())
}
