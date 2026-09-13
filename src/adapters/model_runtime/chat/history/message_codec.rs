use rig::{
    OneOrMany,
    message::{
        AssistantContent, Message, ToolCall, ToolFunction, ToolResult, ToolResultContent,
        UserContent,
    },
};

use crate::entities::conversation::conversation_message::ConversationMessage;

/// Converts provider messages to the bounded domain message vocabulary used by contracts.
pub(crate) fn from_provider(messages: &[Message]) -> Vec<ConversationMessage> {
    messages.iter().flat_map(from_message).collect()
}

/// Converts domain messages back to provider messages at the runtime boundary.
pub(crate) fn to_provider(messages: &[ConversationMessage]) -> Vec<Message> {
    messages.iter().flat_map(to_message).collect()
}

fn from_message(message: &Message) -> Vec<ConversationMessage> {
    match message {
        Message::System { content } => vec![ConversationMessage::System(content.clone())],
        Message::User { content } => content.iter().filter_map(from_user_content).collect(),
        Message::Assistant { content, .. } => {
            content.iter().filter_map(from_assistant_content).collect()
        }
    }
}

fn from_user_content(content: &UserContent) -> Option<ConversationMessage> {
    match content {
        UserContent::Text(text) => Some(ConversationMessage::User(text.text.clone())),
        UserContent::ToolResult(result) => Some(ConversationMessage::ToolResult {
            id: result.id.clone(),
            content: format_tool_result(result),
        }),
        _ => None,
    }
}

fn from_assistant_content(content: &AssistantContent) -> Option<ConversationMessage> {
    match content {
        AssistantContent::Text(text) => match &text.additional_params {
            Some(metadata) => Some(ConversationMessage::AssistantWithMetadata {
                text: text.text.clone(),
                metadata: metadata.clone(),
            }),
            None => Some(ConversationMessage::Assistant(text.text.clone())),
        },
        AssistantContent::ToolCall(call) => Some(ConversationMessage::AssistantToolCall {
            id: call.id.clone(),
            name: call.function.name.clone(),
            arguments: call.function.arguments.to_string(),
        }),
        _ => None,
    }
}

fn format_tool_result(result: &ToolResult) -> String {
    result
        .content
        .iter()
        .map(|content| match content {
            ToolResultContent::Text(text) => text.text.clone(),
            ToolResultContent::Json { value } => value.to_string(),
            ToolResultContent::Image(_) => "<image>".to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn to_message(message: &ConversationMessage) -> Vec<Message> {
    match message {
        ConversationMessage::System(text) => vec![Message::System {
            content: text.clone(),
        }],
        ConversationMessage::User(text) => vec![Message::user(text)],
        ConversationMessage::Assistant(text) => vec![Message::assistant(text)],
        ConversationMessage::AssistantWithMetadata { text, metadata } => {
            vec![Message::Assistant {
                id: None,
                content: OneOrMany::one(AssistantContent::Text(rig::completion::message::Text {
                    text: text.clone(),
                    additional_params: Some(metadata.clone()),
                })),
            }]
        }
        ConversationMessage::AssistantToolCall {
            id,
            name,
            arguments,
        } => vec![Message::Assistant {
            id: None,
            content: OneOrMany::one(AssistantContent::ToolCall(ToolCall::new(
                id.clone(),
                ToolFunction::new(
                    name.clone(),
                    serde_json::from_str(arguments)
                        .unwrap_or_else(|_| serde_json::Value::String(arguments.clone())),
                ),
            ))),
        }],
        ConversationMessage::ToolResult { id, content } => vec![Message::User {
            content: OneOrMany::one(UserContent::ToolResult(ToolResult {
                id: id.clone(),
                call_id: None,
                content: OneOrMany::one(ToolResultContent::text(content)),
            })),
        }],
    }
}
