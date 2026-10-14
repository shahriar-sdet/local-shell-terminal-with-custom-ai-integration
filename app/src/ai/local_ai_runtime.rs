//! The client for the user's own AI endpoint, rebuilt whenever its settings change.
//!
//! This model also owns the process-wide network allowlist: while AI is enabled it contains
//! exactly the configured endpoint, and otherwise nothing, so no other host can ever be reached.

use std::sync::{Arc, RwLock};

use local_ai::{LocalAiClient, LocalAiConfig, LocalAiError, ModelInfo};
use settings::Setting as _;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

use crate::settings::{AISettings, AISettingsChangedEvent};

/// A consistent pair of the endpoint configuration and the client built from it.
#[derive(Clone)]
pub(crate) struct LocalAiSnapshot {
    pub client: Arc<LocalAiClient>,
    pub config: LocalAiConfig,
}

static CURRENT: RwLock<Option<LocalAiSnapshot>> = RwLock::new(None);
/// Ids of the chat models the endpoint last reported.
static KNOWN_MODELS: RwLock<Vec<String>> = RwLock::new(Vec::new());

/// The configured endpoint, for code running off the main thread. `None` while AI is turned off
/// or the endpoint URL is invalid.
pub(crate) fn current() -> Option<LocalAiSnapshot> {
    CURRENT
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// Whether the endpoint offers a chat model with this id.
pub(crate) fn is_known_model(id: &str) -> bool {
    KNOWN_MODELS
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .any(|known| known == id)
}

pub(crate) fn current_or_error() -> Result<LocalAiSnapshot, LocalAiError> {
    current()
        .filter(|snapshot| snapshot.config.is_configured())
        .ok_or(LocalAiError::NotConfigured)
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ModelListState {
    NotLoaded,
    Loading,
    Loaded(Vec<ModelInfo>),
    Failed(String),
}

pub(crate) enum LocalAiRuntimeEvent {
    ModelsChanged,
}

pub(crate) struct LocalAiRuntime {
    models: ModelListState,
    /// Incremented per model-list request so a slow response for an old endpoint is ignored.
    request_generation: u64,
}

impl LocalAiRuntime {
    pub(crate) fn new(ctx: &mut ModelContext<Self>) -> Self {
        ctx.subscribe_to_model(&AISettings::handle(ctx), |me, _, event, ctx| {
            if matches!(
                event,
                AISettingsChangedEvent::LocalAi { .. }
                    | AISettingsChangedEvent::IsAnyAIEnabled { .. }
            ) {
                me.apply_settings(ctx);
            }
        });
        let mut me = Self {
            models: ModelListState::NotLoaded,
            request_generation: 0,
        };
        me.apply_settings(ctx);
        me
    }

    pub(crate) fn models(&self) -> &ModelListState {
        &self.models
    }

    /// Model identifiers suitable for chat, in the order the endpoint lists them.
    pub(crate) fn chat_model_ids(&self) -> Vec<String> {
        match &self.models {
            ModelListState::Loaded(models) => models.iter().map(|model| model.id.clone()).collect(),
            ModelListState::NotLoaded | ModelListState::Loading | ModelListState::Failed(_) => {
                Vec::new()
            }
        }
    }

    /// Reloads the endpoint's model list.
    pub(crate) fn refresh_models(&mut self, ctx: &mut ModelContext<Self>) {
        self.request_generation += 1;
        let Some(snapshot) = current() else {
            self.models = ModelListState::NotLoaded;
            ctx.emit(LocalAiRuntimeEvent::ModelsChanged);
            return;
        };
        let generation = self.request_generation;
        self.models = ModelListState::Loading;
        ctx.emit(LocalAiRuntimeEvent::ModelsChanged);
        ctx.spawn(
            async move { snapshot.client.list_models().await },
            move |me, result, ctx| {
                if me.request_generation != generation {
                    return;
                }
                me.models = match result {
                    Ok(models) => ModelListState::Loaded(
                        models
                            .into_iter()
                            .filter(|model| !model.is_embedding)
                            .collect(),
                    ),
                    Err(error) => ModelListState::Failed(error.to_string()),
                };
                *KNOWN_MODELS
                    .write()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = me.chat_model_ids();
                ctx.emit(LocalAiRuntimeEvent::ModelsChanged);
            },
        );
    }

    fn apply_settings(&mut self, ctx: &mut ModelContext<Self>) {
        let settings = AISettings::as_ref(ctx);
        let ai_enabled = *settings.is_any_ai_enabled;
        let config = settings.local_ai.value().clone();
        let base_url = config.parsed_base_url();

        let snapshot = if ai_enabled {
            match LocalAiClient::new(&config) {
                Ok(client) => Some(LocalAiSnapshot {
                    client: Arc::new(client),
                    config: config.clone(),
                }),
                Err(error) => {
                    log::warn!("AI endpoint is not usable: {error}");
                    None
                }
            }
        } else {
            None
        };
        egress_guard::set_allowed_urls(base_url.iter().filter(|_| snapshot.is_some()));

        let endpoint = |snapshot: &Option<LocalAiSnapshot>| {
            snapshot.as_ref().map(|snapshot| {
                (
                    snapshot.config.provider,
                    snapshot.config.base_url.clone(),
                    snapshot.config.api_key.clone(),
                )
            })
        };
        let endpoint_changed = endpoint(&current()) != endpoint(&snapshot);
        *CURRENT
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = snapshot;
        if endpoint_changed {
            self.refresh_models(ctx);
        }
    }
}

/// Applies `update` to the stored endpoint configuration.
pub(crate) fn update_config(
    update: impl FnOnce(&mut LocalAiConfig),
    ctx: &mut AppContext,
) -> anyhow::Result<()> {
    AISettings::handle(ctx).update(ctx, |settings, ctx| {
        let mut config = settings.local_ai.value().clone();
        update(&mut config);
        settings.local_ai.set_value(config, ctx)
    })
}

impl Entity for LocalAiRuntime {
    type Event = LocalAiRuntimeEvent;
}

impl SingletonEntity for LocalAiRuntime {}
