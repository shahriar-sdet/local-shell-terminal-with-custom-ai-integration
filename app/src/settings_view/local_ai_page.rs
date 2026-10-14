//! Settings for the AI endpoint LocalShell talks to: the provider, its credentials, and the model
//! and generation budget used for each kind of AI task.

use std::collections::HashMap;

use local_ai::{LocalAiConfig, ProviderKind, ReasoningEffort, TaskKind};
use settings::Setting as _;
use warp_core::settings::ToggleableSetting as _;
use warpui::elements::{
    ChildView, Container, Element, Flex, MouseStateHandle, ParentElement, Text,
};
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::ui_components::switch::SwitchStateHandle;
use warpui::{AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle};

use super::ai_shared::render_ai_feature_switch;
use super::settings_page::{
    LocalOnlyIconState, MatchData, PageTitle, PageType, SettingsPageMeta, SettingsPageViewHandle,
    SettingsWidget, render_body_item, render_sub_header_with_description,
};
use super::{SettingsSection, ToggleState};
use crate::ai::local_ai_runtime::{self, LocalAiRuntime, LocalAiRuntimeEvent, ModelListState};
use crate::appearance::Appearance;
use crate::editor::{
    EditorView, Event as EditorEvent, PropagateAndNoOpNavigationKeys, SingleLineEditorOptions,
};
use crate::settings::{AISettings, AISettingsChangedEvent};
use crate::view_components::{Dropdown, DropdownItem};

const TEXT_INPUT_WIDTH: f32 = 320.;
const NUMBER_INPUT_WIDTH: f32 = 120.;
const DROPDOWN_WIDTH: f32 = 320.;
/// Recently refused connections listed under "Network".
const MAX_LISTED_BLOCKED_ATTEMPTS: usize = 5;

#[derive(Clone, Debug, PartialEq)]
pub enum LocalAiPageAction {
    ToggleAiEnabled,
    SetProvider(ProviderKind),
    RefreshModels,
    ToggleManageModelLoading,
    /// `None` uses the agent's model for the task.
    SetTaskModel(TaskKind, Option<String>),
    SetTaskReasoning(TaskKind, ReasoningEffort),
}

/// A validated change to the stored configuration.
type ConfigUpdate = Box<dyn FnOnce(&mut LocalAiConfig)>;

/// A free-text field, committed on Enter or when it loses focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Field {
    BaseUrl,
    ApiKey,
    ContextLength(TaskKind),
    Temperature(TaskKind),
    MaxOutputTokens(TaskKind),
}

struct TaskControls {
    model_dropdown: ViewHandle<Dropdown<LocalAiPageAction>>,
    reasoning_dropdown: ViewHandle<Dropdown<LocalAiPageAction>>,
    context_editor: ViewHandle<EditorView>,
    temperature_editor: ViewHandle<EditorView>,
    max_output_editor: ViewHandle<EditorView>,
}

pub struct LocalAiPageView {
    page: PageType<Self>,
    provider_dropdown: ViewHandle<Dropdown<LocalAiPageAction>>,
    base_url_editor: ViewHandle<EditorView>,
    api_key_editor: ViewHandle<EditorView>,
    tasks: HashMap<TaskKind, TaskControls>,
    field_errors: HashMap<Field, String>,
}

impl LocalAiPageView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let provider_dropdown = Self::new_dropdown(ctx);
        provider_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_items(
                ProviderKind::ALL
                    .into_iter()
                    .map(|provider| {
                        DropdownItem::new(
                            provider.display_name(),
                            LocalAiPageAction::SetProvider(provider),
                        )
                    })
                    .collect(),
                ctx,
            );
        });
        let base_url_editor =
            Self::new_editor(Field::BaseUrl, false, "http://localhost:1234/v1", ctx);
        let api_key_editor = Self::new_editor(Field::ApiKey, true, "Optional for LM Studio", ctx);

        let mut tasks = HashMap::new();
        for task in TaskKind::ALL {
            let reasoning_dropdown = Self::new_dropdown(ctx);
            reasoning_dropdown.update(ctx, |dropdown, ctx| {
                dropdown.set_items(
                    ReasoningEffort::ALL
                        .into_iter()
                        .map(|effort| {
                            DropdownItem::new(
                                effort.display_name(),
                                LocalAiPageAction::SetTaskReasoning(task, effort),
                            )
                        })
                        .collect(),
                    ctx,
                );
            });
            tasks.insert(
                task,
                TaskControls {
                    model_dropdown: Self::new_dropdown(ctx),
                    reasoning_dropdown,
                    context_editor: Self::new_editor(
                        Field::ContextLength(task),
                        false,
                        "Server default",
                        ctx,
                    ),
                    temperature_editor: Self::new_editor(
                        Field::Temperature(task),
                        false,
                        "Default",
                        ctx,
                    ),
                    max_output_editor: Self::new_editor(
                        Field::MaxOutputTokens(task),
                        false,
                        "Default",
                        ctx,
                    ),
                },
            );
        }

        ctx.subscribe_to_model(&AISettings::handle(ctx), |me, _, event, ctx| {
            if matches!(
                event,
                AISettingsChangedEvent::LocalAi { .. }
                    | AISettingsChangedEvent::IsAnyAIEnabled { .. }
            ) {
                me.sync_from_settings(ctx);
            }
        });
        ctx.subscribe_to_model(
            &LocalAiRuntime::handle(ctx),
            |me, _, event, ctx| match event {
                LocalAiRuntimeEvent::ModelsChanged => {
                    me.sync_model_dropdowns(ctx);
                    ctx.notify();
                }
            },
        );

        let mut widgets: Vec<Box<dyn SettingsWidget<View = Self>>> = vec![
            Box::new(AiEnabledWidget::default()),
            Box::new(ProviderWidget),
            Box::new(EndpointWidget),
            Box::new(ApiKeyWidget),
            Box::new(ModelsWidget::default()),
            Box::new(ModelLoadingWidget::default()),
        ];
        widgets.extend(
            TaskKind::ALL
                .into_iter()
                .map(|task| Box::new(TaskWidget { task }) as Box<dyn SettingsWidget<View = Self>>),
        );
        widgets.push(Box::new(NetworkWidget));

        let mut me = Self {
            page: PageType::new_uncategorized(widgets, Some(PageTitle::new("AI provider"))),
            provider_dropdown,
            base_url_editor,
            api_key_editor,
            tasks,
            field_errors: HashMap::new(),
        };
        me.sync_from_settings(ctx);
        me
    }

    fn new_dropdown(ctx: &mut ViewContext<Self>) -> ViewHandle<Dropdown<LocalAiPageAction>> {
        ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_top_bar_max_width(DROPDOWN_WIDTH);
            dropdown
        })
    }

    fn new_editor(
        field: Field,
        is_password: bool,
        placeholder: &'static str,
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<EditorView> {
        let editor = ctx.add_typed_action_view(move |ctx| {
            let options = SingleLineEditorOptions {
                is_password,
                propagate_and_no_op_vertical_navigation_keys:
                    PropagateAndNoOpNavigationKeys::Always,
                ..Default::default()
            };
            let mut editor = EditorView::single_line(options, ctx);
            editor.set_placeholder_text(placeholder, ctx);
            editor
        });
        ctx.subscribe_to_view(&editor, move |me, _, event, ctx| {
            if matches!(event, EditorEvent::Enter | EditorEvent::Blurred) {
                me.commit_field(field, ctx);
            }
        });
        editor
    }

    fn config(app: &AppContext) -> LocalAiConfig {
        AISettings::as_ref(app).local_ai.value().clone()
    }

    fn editor(&self, field: Field) -> &ViewHandle<EditorView> {
        match field {
            Field::BaseUrl => &self.base_url_editor,
            Field::ApiKey => &self.api_key_editor,
            Field::ContextLength(task) => &self.tasks[&task].context_editor,
            Field::Temperature(task) => &self.tasks[&task].temperature_editor,
            Field::MaxOutputTokens(task) => &self.tasks[&task].max_output_editor,
        }
    }

    /// The text a field shows for the stored configuration.
    fn field_text(field: Field, config: &LocalAiConfig) -> String {
        match field {
            Field::BaseUrl => config.base_url.clone(),
            Field::ApiKey => config.api_key.clone(),
            Field::ContextLength(task) => match config.profile(task).context_length {
                0 => String::new(),
                tokens => tokens.to_string(),
            },
            Field::Temperature(task) => config
                .profile(task)
                .temperature
                .map(|temperature| temperature.to_string())
                .unwrap_or_default(),
            Field::MaxOutputTokens(task) => config
                .profile(task)
                .max_output_tokens
                .map(|tokens| tokens.to_string())
                .unwrap_or_default(),
        }
    }

    fn all_fields() -> impl Iterator<Item = Field> {
        [Field::BaseUrl, Field::ApiKey]
            .into_iter()
            .chain(TaskKind::ALL.into_iter().flat_map(|task| {
                [
                    Field::ContextLength(task),
                    Field::Temperature(task),
                    Field::MaxOutputTokens(task),
                ]
            }))
    }

    /// Validates a field's text and stores it, or records an error and leaves the setting as is.
    fn commit_field(&mut self, field: Field, ctx: &mut ViewContext<Self>) {
        let text = self
            .editor(field)
            .as_ref(ctx)
            .buffer_text(ctx)
            .trim()
            .to_owned();
        let result: Result<ConfigUpdate, String> = match field {
            Field::BaseUrl => {
                let candidate = LocalAiConfig {
                    base_url: text.clone(),
                    ..Default::default()
                };
                if candidate.parsed_base_url().is_some() {
                    Ok(Box::new(move |config: &mut LocalAiConfig| {
                        config.base_url = text
                    }))
                } else {
                    Err(
                        "Enter an http:// or https:// URL, e.g. http://localhost:1234/v1"
                            .to_owned(),
                    )
                }
            }
            Field::ApiKey => Ok(Box::new(move |config: &mut LocalAiConfig| {
                config.api_key = text
            })),
            Field::ContextLength(task) => parse_optional::<u32>(&text)
                .filter(|tokens| tokens.is_none_or(|tokens| tokens >= 512))
                .map(|tokens| -> ConfigUpdate {
                    Box::new(move |config| {
                        config.profile_mut(task).context_length = tokens.unwrap_or(0)
                    })
                })
                .ok_or_else(|| {
                    "Enter a number of tokens of at least 512, or leave empty.".to_owned()
                }),
            Field::Temperature(task) => parse_optional::<f32>(&text)
                .filter(|temperature| temperature.is_none_or(|value| (0.0..=2.0).contains(&value)))
                .map(|temperature| -> ConfigUpdate {
                    Box::new(move |config| config.profile_mut(task).temperature = temperature)
                })
                .ok_or_else(|| "Enter a number between 0 and 2, or leave empty.".to_owned()),
            Field::MaxOutputTokens(task) => parse_optional::<u32>(&text)
                .filter(|tokens| tokens.is_none_or(|tokens| tokens > 0))
                .map(|tokens| -> ConfigUpdate {
                    Box::new(move |config| config.profile_mut(task).max_output_tokens = tokens)
                })
                .ok_or_else(|| "Enter a positive number of tokens, or leave empty.".to_owned()),
        };
        match result {
            Ok(update) => {
                self.field_errors.remove(&field);
                let unchanged = {
                    let current = Self::config(ctx);
                    let mut updated = current.clone();
                    update(&mut updated);
                    if updated == current {
                        true
                    } else {
                        if let Err(error) =
                            local_ai_runtime::update_config(|config| *config = updated, ctx)
                        {
                            log::warn!("Could not save the AI provider settings: {error:#}");
                        }
                        false
                    }
                };
                if unchanged {
                    ctx.notify();
                }
            }
            Err(message) => {
                self.field_errors.insert(field, message);
                ctx.notify();
            }
        }
    }

    /// Brings every control in line with the stored configuration.
    fn sync_from_settings(&mut self, ctx: &mut ViewContext<Self>) {
        let config = Self::config(ctx);
        self.provider_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_selected_by_action(LocalAiPageAction::SetProvider(config.provider), ctx);
        });
        for field in Self::all_fields() {
            // A field showing a validation error keeps the user's text so they can fix it.
            if self.field_errors.contains_key(&field) {
                continue;
            }
            let text = Self::field_text(field, &config);
            self.editor(field).update(ctx, |editor, ctx| {
                if editor.buffer_text(ctx) != text {
                    editor.set_buffer_text(&text, ctx);
                }
            });
        }
        for task in TaskKind::ALL {
            let effort = config.profile(task).reasoning;
            self.tasks[&task]
                .reasoning_dropdown
                .update(ctx, |dropdown, ctx| {
                    dropdown.set_selected_by_action(
                        LocalAiPageAction::SetTaskReasoning(task, effort),
                        ctx,
                    );
                });
        }
        self.sync_model_dropdowns(ctx);
        ctx.notify();
    }

    fn sync_model_dropdowns(&mut self, ctx: &mut ViewContext<Self>) {
        let config = Self::config(ctx);
        let model_ids = LocalAiRuntime::as_ref(ctx).chat_model_ids();
        for task in TaskKind::ALL {
            let selected = config.profile(task).model.trim().to_owned();
            let mut items = Vec::new();
            if task != TaskKind::Agent {
                items.push(DropdownItem::new(
                    "Same as agent",
                    LocalAiPageAction::SetTaskModel(task, None),
                ));
            }
            let mut ids = model_ids.clone();
            if !selected.is_empty() && !ids.contains(&selected) {
                ids.insert(0, selected.clone());
            }
            if ids.is_empty() && task == TaskKind::Agent {
                items.push(DropdownItem::new(
                    "No models found — check the endpoint",
                    LocalAiPageAction::RefreshModels,
                ));
            }
            items.extend(ids.into_iter().map(|id| {
                DropdownItem::new(id.clone(), LocalAiPageAction::SetTaskModel(task, Some(id)))
            }));
            let selected_action = if selected.is_empty() {
                (task != TaskKind::Agent).then_some(LocalAiPageAction::SetTaskModel(task, None))
            } else {
                Some(LocalAiPageAction::SetTaskModel(task, Some(selected)))
            };
            self.tasks[&task]
                .model_dropdown
                .update(ctx, |dropdown, ctx| {
                    dropdown.set_items(items, ctx);
                    if let Some(action) = selected_action {
                        dropdown.set_selected_by_action(action, ctx);
                    }
                });
        }
    }

    fn render_text_input(
        &self,
        field: Field,
        width: f32,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let input = appearance
            .ui_builder()
            .text_input(self.editor(field).clone())
            .with_style(UiComponentStyles {
                width: Some(width),
                ..Default::default()
            })
            .build()
            .finish();
        let Some(error) = self.field_errors.get(&field) else {
            return input;
        };
        Flex::column()
            .with_child(input)
            .with_child(
                Container::new(
                    Text::new(error.clone(), appearance.ui_font_family(), 12.)
                        .with_color(appearance.theme().ui_error_color())
                        .soft_wrap(true)
                        .finish(),
                )
                .with_margin_top(4.)
                .finish(),
            )
            .finish()
    }
}

/// Parses an optional number: empty text means "unset".
fn parse_optional<T: std::str::FromStr>(text: &str) -> Option<Option<T>> {
    if text.is_empty() {
        Some(None)
    } else {
        text.parse().ok().map(Some)
    }
}

impl Entity for LocalAiPageView {
    type Event = ();
}

impl TypedActionView for LocalAiPageView {
    type Action = LocalAiPageAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        let result = match action {
            LocalAiPageAction::ToggleAiEnabled => AISettings::handle(ctx)
                .update(ctx, |settings, ctx| {
                    settings.is_any_ai_enabled.toggle_and_save_value(ctx)
                })
                .map(|_| ()),
            LocalAiPageAction::SetProvider(provider) => {
                let provider = *provider;
                local_ai_runtime::update_config(
                    |config| {
                        if config.provider == provider {
                            return;
                        }
                        // Keep a custom URL, but follow the default when switching providers.
                        if config.base_url.trim() == config.provider.default_base_url() {
                            config.base_url = provider.default_base_url().to_owned();
                        }
                        config.provider = provider;
                    },
                    ctx,
                )
            }
            LocalAiPageAction::RefreshModels => {
                LocalAiRuntime::handle(ctx).update(ctx, |runtime, ctx| runtime.refresh_models(ctx));
                Ok(())
            }
            LocalAiPageAction::ToggleManageModelLoading => local_ai_runtime::update_config(
                |config| config.manage_model_loading = !config.manage_model_loading,
                ctx,
            ),
            LocalAiPageAction::SetTaskModel(task, model) => {
                let (task, model) = (*task, model.clone().unwrap_or_default());
                local_ai_runtime::update_config(
                    |config| config.profile_mut(task).model = model,
                    ctx,
                )
            }
            LocalAiPageAction::SetTaskReasoning(task, effort) => {
                let (task, effort) = (*task, *effort);
                local_ai_runtime::update_config(
                    |config| config.profile_mut(task).reasoning = effort,
                    ctx,
                )
            }
        };
        if let Err(error) = result {
            log::warn!("Could not save the AI provider settings: {error:#}");
        }
        ctx.notify();
    }
}

impl View for LocalAiPageView {
    fn ui_name() -> &'static str {
        "LocalAiSettingsPage"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        self.page.render(self, app)
    }
}

impl SettingsPageMeta for LocalAiPageView {
    fn section() -> SettingsSection {
        SettingsSection::LocalAi
    }

    fn should_render(&self, _ctx: &AppContext) -> bool {
        true
    }

    fn update_filter(&mut self, query: &str, ctx: &mut ViewContext<Self>) -> MatchData {
        self.page.update_filter(query, ctx)
    }

    fn scroll_to_widget(&mut self, widget_id: &'static str) {
        self.page.scroll_to_widget(widget_id)
    }

    fn clear_highlighted_widget(&mut self) {
        self.page.clear_highlighted_widget();
    }
}

impl From<ViewHandle<LocalAiPageView>> for SettingsPageViewHandle {
    fn from(view_handle: ViewHandle<LocalAiPageView>) -> Self {
        SettingsPageViewHandle::LocalAi(view_handle)
    }
}

fn body_row(
    label: &str,
    control: Box<dyn Element>,
    description: Option<String>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    render_body_item::<LocalAiPageAction>(
        label.to_owned(),
        None,
        LocalOnlyIconState::Hidden,
        ToggleState::Enabled,
        appearance,
        control,
        description,
    )
}

#[derive(Default)]
struct AiEnabledWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for AiEnabledWidget {
    type View = LocalAiPageView;

    fn search_terms(&self) -> &str {
        "ai features enable disable turn off on agent local"
    }

    fn render(
        &self,
        _view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let enabled = *AISettings::as_ref(app).is_any_ai_enabled;
        body_row(
            "AI features",
            render_ai_feature_switch(
                self.switch_state.clone(),
                enabled,
                true,
                LocalAiPageAction::ToggleAiEnabled,
                app,
            ),
            Some(
                "LocalShell sends AI requests only to the endpoint below; nothing goes to Warp or \
                 any other server. While this is off, LocalShell makes no network connections at all."
                    .to_owned(),
            ),
            appearance,
        )
    }
}

struct ProviderWidget;

impl SettingsWidget for ProviderWidget {
    type View = LocalAiPageView;

    fn search_terms(&self) -> &str {
        "ai provider lm studio openai compatible ollama local model server"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        _app: &AppContext,
    ) -> Box<dyn Element> {
        body_row(
            "Provider",
            ChildView::new(&view.provider_dropdown).finish(),
            Some(
                "LM Studio adds model load control. OpenAI-compatible works with OpenAI, Ollama, \
                 llama.cpp, vLLM or a company gateway."
                    .to_owned(),
            ),
            appearance,
        )
    }
}

struct EndpointWidget;

impl SettingsWidget for EndpointWidget {
    type View = LocalAiPageView;

    fn search_terms(&self) -> &str {
        "ai provider endpoint url base address host port server"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        _app: &AppContext,
    ) -> Box<dyn Element> {
        body_row(
            "Endpoint URL",
            view.render_text_input(Field::BaseUrl, TEXT_INPUT_WIDTH, appearance),
            Some(
                "The base URL including /v1. This is the only host LocalShell will connect to."
                    .to_owned(),
            ),
            appearance,
        )
    }
}

struct ApiKeyWidget;

impl SettingsWidget for ApiKeyWidget {
    type View = LocalAiPageView;

    fn search_terms(&self) -> &str {
        "ai provider api key token credential secret bearer"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        _app: &AppContext,
    ) -> Box<dyn Element> {
        body_row(
            "API key",
            view.render_text_input(Field::ApiKey, TEXT_INPUT_WIDTH, appearance),
            Some(
                "Sent as a bearer token to the endpoint. Stored in your settings file.".to_owned(),
            ),
            appearance,
        )
    }
}

#[derive(Default)]
struct ModelsWidget {
    refresh_button_state: MouseStateHandle,
}

impl SettingsWidget for ModelsWidget {
    type View = LocalAiPageView;

    fn search_terms(&self) -> &str {
        "ai provider models refresh test connection available"
    }

    fn render(
        &self,
        _view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ai_enabled = *AISettings::as_ref(app).is_any_ai_enabled;
        let status = if !ai_enabled {
            "Turn on AI features to connect to the endpoint.".to_owned()
        } else {
            match LocalAiRuntime::as_ref(app).models() {
                ModelListState::NotLoaded => "Not connected yet.".to_owned(),
                ModelListState::Loading => "Connecting…".to_owned(),
                ModelListState::Loaded(models) => {
                    format!("Connected. {} chat models available.", models.len())
                }
                ModelListState::Failed(error) => error.clone(),
            }
        };
        let mut button = appearance
            .ui_builder()
            .button(ButtonVariant::Secondary, self.refresh_button_state.clone())
            .with_text_label("Refresh models".to_owned());
        if !ai_enabled {
            button = button.disabled();
        }
        let button = button
            .build()
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(LocalAiPageAction::RefreshModels))
            .finish();
        body_row("Models", button, Some(status), appearance)
    }
}

#[derive(Default)]
struct ModelLoadingWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for ModelLoadingWidget {
    type View = LocalAiPageView;

    fn search_terms(&self) -> &str {
        "lm studio load model context length automatically prepare"
    }

    fn should_render(&self, app: &AppContext) -> bool {
        LocalAiPageView::config(app).provider == ProviderKind::LmStudio
    }

    fn render(
        &self,
        _view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        body_row(
            "Load models for each task",
            render_ai_feature_switch(
                self.switch_state.clone(),
                LocalAiPageView::config(app).manage_model_loading,
                true,
                LocalAiPageAction::ToggleManageModelLoading,
                app,
            ),
            Some(
                "Before a request, LM Studio loads the task's model with the task's context length \
                 unless it is already loaded with at least that much."
                    .to_owned(),
            ),
            appearance,
        )
    }
}

struct TaskWidget {
    task: TaskKind,
}

impl SettingsWidget for TaskWidget {
    type View = LocalAiPageView;

    fn search_terms(&self) -> &str {
        match self.task {
            TaskKind::Agent => {
                "agent model context length reasoning thinking temperature max output tokens"
            }
            TaskKind::CommandGeneration => {
                "command generation search natural language model context reasoning thinking \
                 temperature max output tokens"
            }
            TaskKind::Suggestions => {
                "suggestions autosuggestions next command prompt model context reasoning \
                 thinking temperature max output tokens"
            }
            TaskKind::Utility => {
                "titles summaries utility model context reasoning thinking temperature max \
                 output tokens"
            }
        }
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        _app: &AppContext,
    ) -> Box<dyn Element> {
        let task = self.task;
        let controls = &view.tasks[&task];
        Flex::column()
            .with_child(render_sub_header_with_description(
                appearance,
                task.display_name(),
                task.description(),
            ))
            .with_child(body_row(
                "Model",
                ChildView::new(&controls.model_dropdown).finish(),
                None,
                appearance,
            ))
            .with_child(body_row(
                "Reasoning",
                ChildView::new(&controls.reasoning_dropdown).finish(),
                Some("How much the model thinks before answering, if it supports it.".to_owned()),
                appearance,
            ))
            .with_child(body_row(
                "Context length",
                view.render_text_input(Field::ContextLength(task), NUMBER_INPUT_WIDTH, appearance),
                Some("Tokens of context. LM Studio loads the model with this much.".to_owned()),
                appearance,
            ))
            .with_child(body_row(
                "Temperature",
                view.render_text_input(Field::Temperature(task), NUMBER_INPUT_WIDTH, appearance),
                None,
                appearance,
            ))
            .with_child(body_row(
                "Max output tokens",
                view.render_text_input(
                    Field::MaxOutputTokens(task),
                    NUMBER_INPUT_WIDTH,
                    appearance,
                ),
                None,
                appearance,
            ))
            .finish()
    }
}

struct NetworkWidget;

impl SettingsWidget for NetworkWidget {
    type View = LocalAiPageView;

    fn search_terms(&self) -> &str {
        "network privacy blocked connections allowed host egress compliance"
    }

    fn render(
        &self,
        _view: &Self::View,
        appearance: &Appearance,
        _app: &AppContext,
    ) -> Box<dyn Element> {
        let allowed = egress_guard::allowed_origins();
        let mut description = if allowed.is_empty() {
            "No outbound connections are allowed.".to_owned()
        } else {
            format!("Only {} can be reached.", allowed.join(", "))
        };
        let blocked = egress_guard::recent_blocked_attempts();
        if blocked.is_empty() {
            description.push_str(" No connection attempts have been blocked.");
        } else {
            let recent: Vec<&str> = blocked
                .iter()
                .rev()
                .take(MAX_LISTED_BLOCKED_ATTEMPTS)
                .map(|attempt| attempt.target.as_str())
                .collect();
            description.push_str(&format!(
                " Blocked {} connection attempts since launch, most recently to {}.",
                blocked.len(),
                recent.join(", ")
            ));
        }
        body_row(
            "Network",
            Container::new(Flex::row().finish()).finish(),
            Some(description),
            appearance,
        )
    }
}
