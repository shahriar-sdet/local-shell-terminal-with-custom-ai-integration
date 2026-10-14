//! Conversions from request inputs to the task messages that record them in the conversation.
//! Warp's server performs these when it echoes inputs back; the client only re-sends history it
//! received as task messages.

use warp_multi_agent_api as api;
use warp_multi_agent_api::request::input as req;

pub(crate) fn user_query_message(
    query: req::UserQuery,
    context: Option<api::InputContext>,
) -> api::message::Message {
    api::message::Message::UserQuery(api::message::UserQuery {
        query: query.query,
        context,
        referenced_attachments: query.referenced_attachments,
        mode: query.mode,
        intended_agent: query.intended_agent,
        origin: query.origin,
        author: query.author,
        source_message: query.source_message,
    })
}

/// A synthesized user turn for inputs that aren't typed queries (e.g. "resume", "init project").
pub(crate) fn text_query_message(
    text: String,
    context: Option<api::InputContext>,
) -> api::message::Message {
    api::message::Message::UserQuery(api::message::UserQuery {
        query: text,
        context,
        ..Default::default()
    })
}

pub(crate) fn tool_call_result_message(
    result: req::ToolCallResult,
    context: Option<api::InputContext>,
) -> api::message::Message {
    api::message::Message::ToolCallResult(api::message::ToolCallResult {
        tool_call_id: result.tool_call_id,
        context,
        result: result.result.map(convert_result),
    })
}

#[allow(deprecated)]
fn convert_result(result: req::tool_call_result::Result) -> api::message::tool_call_result::Result {
    use api::message::tool_call_result::Result as Message;
    use req::tool_call_result::Result as Request;

    // The two oneofs carry the same result types under the same variant names, except for the
    // computer-use request whose message-side variant has a `Result` suffix.
    macro_rules! same_variants {
        ($($variant:ident),* $(,)?) => {
            match result {
                $(Request::$variant(inner) => Message::$variant(inner),)*
                Request::RequestComputerUse(inner) => Message::RequestComputerUseResult(inner),
            }
        };
    }
    same_variants!(
        RunShellCommand,
        ReadFiles,
        SearchCodebase,
        ApplyFileDiffs,
        SuggestPlan,
        SuggestCreatePlan,
        Grep,
        FileGlob,
        ReadMcpResource,
        CallMcpTool,
        WriteToLongRunningShellCommand,
        SuggestNewConversation,
        FileGlobV2,
        SuggestPrompt,
        OpenCodeReview,
        InitProject,
        ReadDocuments,
        EditDocuments,
        CreateDocuments,
        ReadShellCommandOutput,
        UseComputer,
        InsertReviewComments,
        ReadSkill,
        FetchConversation,
        SendMessageToAgent,
        TransferShellCommandControlToUser,
        AskUserQuestion,
        UploadFileArtifact,
        RunAgentsResult,
        WaitForEvents,
        StartRecording,
        StopRecording,
    )
}
