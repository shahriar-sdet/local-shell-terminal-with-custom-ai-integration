use serde::{Deserialize, Serialize};
use url::Url;

/// The kind of server LocalShell sends AI requests to.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// LM Studio's local server. Enables model listing with load state and per-task model loading.
    #[default]
    LmStudio,
    /// Any server implementing OpenAI's `/v1/chat/completions` API (OpenAI, Ollama, llama.cpp,
    /// vLLM, a company gateway, ...).
    OpenAiCompatible,
}

impl ProviderKind {
    pub const ALL: [ProviderKind; 2] = [ProviderKind::LmStudio, ProviderKind::OpenAiCompatible];

    pub fn display_name(self) -> &'static str {
        match self {
            ProviderKind::LmStudio => "LM Studio",
            ProviderKind::OpenAiCompatible => "OpenAI-compatible",
        }
    }

    pub fn default_base_url(self) -> &'static str {
        match self {
            ProviderKind::LmStudio => "http://localhost:1234/v1",
            ProviderKind::OpenAiCompatible => "https://api.openai.com/v1",
        }
    }
}

/// An AI workload with its own model and generation budget, so cheap tasks (suggesting a shell
/// command) don't pay for the context window and reasoning that agent conversations need.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    /// Agent Mode conversations: multi-step work with tool calls.
    Agent,
    /// Natural language to shell command, and command explanations.
    CommandGeneration,
    /// Next-command and prompt suggestions computed while the user types.
    Suggestions,
    /// Short background generations: titles, summaries, commit messages.
    Utility,
}

impl TaskKind {
    pub const ALL: [TaskKind; 4] = [
        TaskKind::Agent,
        TaskKind::CommandGeneration,
        TaskKind::Suggestions,
        TaskKind::Utility,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            TaskKind::Agent => "Agent",
            TaskKind::CommandGeneration => "Command generation",
            TaskKind::Suggestions => "Suggestions",
            TaskKind::Utility => "Titles & summaries",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            TaskKind::Agent => {
                "Agent Mode conversations that run commands, read and edit files. Needs a model \
                 trained for tool use and a large context."
            }
            TaskKind::CommandGeneration => {
                "Turning plain English into a shell command and explaining commands. A small, \
                 fast model with reasoning off works well."
            }
            TaskKind::Suggestions => {
                "Next-command and prompt suggestions while you type. Keep this fast: small \
                 context, short output, no reasoning."
            }
            TaskKind::Utility => {
                "Background text such as conversation titles, block summaries and commit \
                 messages."
            }
        }
    }
}

/// How much the model should think before answering.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    /// Don't send a reasoning setting; the model uses its own default.
    #[default]
    ModelDefault,
    Off,
    Low,
    Medium,
    High,
}

impl ReasoningEffort {
    pub const ALL: [ReasoningEffort; 5] = [
        ReasoningEffort::ModelDefault,
        ReasoningEffort::Off,
        ReasoningEffort::Low,
        ReasoningEffort::Medium,
        ReasoningEffort::High,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            ReasoningEffort::ModelDefault => "Model default",
            ReasoningEffort::Off => "Off",
            ReasoningEffort::Low => "Low",
            ReasoningEffort::Medium => "Medium",
            ReasoningEffort::High => "High",
        }
    }
}

/// Model and generation settings for one [`TaskKind`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct TaskProfile {
    /// Model identifier as reported by the provider. Empty means "same model as the agent".
    pub model: String,
    /// Context window, in tokens, to load the model with in LM Studio. `0` keeps the server's
    /// default. Also used to trim conversation history before it overflows the window.
    pub context_length: u32,
    /// Sampling temperature. `None` uses the model default; OpenAI reasoning models reject any
    /// explicit value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// Upper bound on generated tokens. `None` uses the server default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    pub reasoning: ReasoningEffort,
}

impl Default for TaskProfile {
    fn default() -> Self {
        Self::preset(TaskKind::Agent)
    }
}

impl TaskProfile {
    /// Sensible starting values for each task on a typical Apple Silicon Mac.
    pub fn preset(task: TaskKind) -> Self {
        match task {
            TaskKind::Agent => Self {
                model: String::new(),
                context_length: 32_768,
                temperature: None,
                max_output_tokens: None,
                reasoning: ReasoningEffort::Medium,
            },
            TaskKind::CommandGeneration => Self {
                model: String::new(),
                context_length: 4_096,
                temperature: Some(0.2),
                max_output_tokens: Some(512),
                reasoning: ReasoningEffort::Off,
            },
            TaskKind::Suggestions => Self {
                model: String::new(),
                context_length: 4_096,
                temperature: Some(0.2),
                max_output_tokens: Some(128),
                reasoning: ReasoningEffort::Off,
            },
            TaskKind::Utility => Self {
                model: String::new(),
                context_length: 4_096,
                temperature: Some(0.3),
                max_output_tokens: Some(256),
                reasoning: ReasoningEffort::Off,
            },
        }
    }
}

/// The user's AI provider configuration. Stored in `settings.toml` under `agents.local_ai`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct LocalAiConfig {
    pub provider: ProviderKind,
    /// Base URL of the OpenAI-compatible API, including the version segment (e.g.
    /// `http://localhost:1234/v1`). This is the only host LocalShell will ever connect to.
    pub base_url: String,
    /// Sent as a bearer token when non-empty. LM Studio only needs one when its server has
    /// authentication turned on.
    pub api_key: String,
    /// LM Studio only: before a request, load the task's model with the task's context length
    /// unless it is already loaded with at least that much.
    pub manage_model_loading: bool,
    pub agent: TaskProfile,
    pub command_generation: TaskProfile,
    pub suggestions: TaskProfile,
    pub utility: TaskProfile,
}

impl Default for LocalAiConfig {
    fn default() -> Self {
        Self {
            provider: ProviderKind::default(),
            base_url: ProviderKind::default().default_base_url().to_owned(),
            api_key: String::new(),
            manage_model_loading: true,
            agent: TaskProfile::preset(TaskKind::Agent),
            command_generation: TaskProfile::preset(TaskKind::CommandGeneration),
            suggestions: TaskProfile::preset(TaskKind::Suggestions),
            utility: TaskProfile::preset(TaskKind::Utility),
        }
    }
}

impl settings_value::SettingsValue for LocalAiConfig {}

impl LocalAiConfig {
    /// Parses [`Self::base_url`], accepting only `http` and `https`.
    pub fn parsed_base_url(&self) -> Option<Url> {
        let mut url = Url::parse(self.base_url.trim()).ok()?;
        if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
            return None;
        }
        // `Url::join` replaces the last path segment unless the path ends with a slash.
        if !url.path().ends_with('/') {
            let path = format!("{}/", url.path());
            url.set_path(&path);
        }
        Some(url)
    }

    /// Whether AI requests can be sent: the endpoint parses and the agent has a model.
    pub fn is_configured(&self) -> bool {
        self.parsed_base_url().is_some() && !self.agent.model.trim().is_empty()
    }

    pub fn profile(&self, task: TaskKind) -> &TaskProfile {
        match task {
            TaskKind::Agent => &self.agent,
            TaskKind::CommandGeneration => &self.command_generation,
            TaskKind::Suggestions => &self.suggestions,
            TaskKind::Utility => &self.utility,
        }
    }

    pub fn profile_mut(&mut self, task: TaskKind) -> &mut TaskProfile {
        match task {
            TaskKind::Agent => &mut self.agent,
            TaskKind::CommandGeneration => &mut self.command_generation,
            TaskKind::Suggestions => &mut self.suggestions,
            TaskKind::Utility => &mut self.utility,
        }
    }

    /// The model to use for `task`, falling back to the agent model when the task has none.
    pub fn model_for(&self, task: TaskKind) -> &str {
        let model = self.profile(task).model.trim();
        if model.is_empty() {
            self.agent.model.trim()
        } else {
            model
        }
    }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
