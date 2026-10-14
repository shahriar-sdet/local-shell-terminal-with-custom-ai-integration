//! Builders for the response events the Warp client consumes.

use std::time::SystemTime;

use prost_types::FieldMask;
use uuid::Uuid;
use warp_multi_agent_api as api;
use warp_multi_agent_api::client_action::{
    Action, AddMessagesToTask, AppendToMessageContent, CreateTask, UpdateTaskMessage,
};
use warp_multi_agent_api::response_event::{self, StreamFinished, StreamInit, stream_finished};

pub(crate) const AGENT_OUTPUT_TEXT: &str = "agent_output.text";
pub(crate) const AGENT_REASONING_TEXT: &str = "agent_reasoning.reasoning";
pub(crate) const AGENT_REASONING_DURATION: &str = "agent_reasoning.finished_duration";

pub(crate) fn init(conversation_id: &str, request_id: &str) -> api::ResponseEvent {
    api::ResponseEvent {
        r#type: Some(response_event::Type::Init(StreamInit {
            conversation_id: conversation_id.to_owned(),
            request_id: request_id.to_owned(),
            run_id: String::new(),
        })),
    }
}

pub(crate) fn actions(actions: Vec<Action>) -> api::ResponseEvent {
    api::ResponseEvent {
        r#type: Some(response_event::Type::ClientActions(
            response_event::ClientActions {
                actions: actions
                    .into_iter()
                    .map(|action| api::ClientAction {
                        action: Some(action),
                    })
                    .collect(),
            },
        )),
    }
}

pub(crate) fn finished(
    reason: stream_finished::Reason,
    usage: Option<stream_finished::ConversationUsageMetadata>,
) -> api::ResponseEvent {
    api::ResponseEvent {
        r#type: Some(response_event::Type::Finished(StreamFinished {
            reason: Some(reason),
            conversation_usage_metadata: usage,
            ..Default::default()
        })),
    }
}

pub(crate) fn done() -> stream_finished::Reason {
    stream_finished::Reason::Done(stream_finished::Done {})
}

pub(crate) fn internal_error(message: impl Into<String>) -> stream_finished::Reason {
    stream_finished::Reason::InternalError(stream_finished::InternalError {
        message: message.into(),
    })
}

pub(crate) fn create_root_task(task_id: &str) -> Action {
    Action::CreateTask(CreateTask {
        task: Some(api::Task {
            id: task_id.to_owned(),
            ..Default::default()
        }),
    })
}

pub(crate) fn add_messages(task_id: &str, messages: Vec<api::Message>) -> Action {
    Action::AddMessagesToTask(AddMessagesToTask {
        task_id: task_id.to_owned(),
        messages,
    })
}

pub(crate) fn append(task_id: &str, message: api::Message, path: &str) -> Action {
    Action::AppendToMessageContent(AppendToMessageContent {
        task_id: task_id.to_owned(),
        message: Some(message),
        mask: Some(FieldMask {
            paths: vec![path.to_owned()],
        }),
    })
}

pub(crate) fn update(task_id: &str, message: api::Message, path: &str) -> Action {
    Action::UpdateTaskMessage(UpdateTaskMessage {
        task_id: task_id.to_owned(),
        message: Some(message),
        mask: Some(FieldMask {
            paths: vec![path.to_owned()],
        }),
    })
}

/// Stamps new messages with the IDs the client expects on every message of a response.
pub(crate) struct MessageFactory {
    pub(crate) task_id: String,
    pub(crate) request_id: String,
}

impl MessageFactory {
    pub(crate) fn new_message(&self, kind: api::message::Message) -> api::Message {
        self.message_with_id(Uuid::new_v4().to_string(), kind)
    }

    pub(crate) fn message_with_id(&self, id: String, kind: api::message::Message) -> api::Message {
        api::Message {
            id,
            task_id: self.task_id.clone(),
            request_id: self.request_id.clone(),
            timestamp: Some(SystemTime::now().into()),
            message: Some(kind),
            ..Default::default()
        }
    }
}
