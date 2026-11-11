use rig::message::Message;

use crate::entities::conversation::conversation_message::ConversationMessage;

/// Converts provider messages to one lossless domain boundary per provider message.
pub(crate) fn from_provider(messages: &[Message]) -> anyhow::Result<Vec<ConversationMessage>> {
    messages
        .iter()
        .map(|message| {
            let raw = serde_json::to_value(message)?;
            Ok(canonical_message(&raw).unwrap_or(ConversationMessage::Provider { raw }))
        })
        .collect()
}

/// Converts domain messages back to provider messages without silently dropping invalid values.
pub(crate) fn to_provider(messages: &[ConversationMessage]) -> anyhow::Result<Vec<Message>> {
    messages.iter().map(to_message).collect()
}

fn to_message(message: &ConversationMessage) -> anyhow::Result<Message> {
    match message {
        ConversationMessage::Provider { raw } => {
            Ok(normalize_message(serde_json::from_value(raw.clone())?))
        }
        ConversationMessage::System(text) => Ok(Message::System {
            content: text.clone(),
        }),
        ConversationMessage::User(text) => Ok(Message::user(text)),
        ConversationMessage::Assistant(text) => Ok(Message::assistant(text)),
    }
}

fn canonical_message(raw: &serde_json::Value) -> Option<ConversationMessage> {
    let role = raw.get("role")?.as_str()?;
    let text = canonical_text(raw, role)?;
    match role {
        "system" => Some(ConversationMessage::System(text)),
        "user" => Some(ConversationMessage::User(text)),
        "assistant" => Some(ConversationMessage::Assistant(text)),
        _ => None,
    }
}

fn canonical_text(raw: &serde_json::Value, role: &str) -> Option<String> {
    let content = raw.get("content")?;
    let text = if role == "system" {
        content.as_str()?.to_string()
    } else {
        let parts = content.as_array()?;
        if parts.len() != 1 {
            return None;
        }
        if role == "user" && parts[0].get("type")?.as_str()? != "text" {
            return None;
        }
        parts[0].get("text")?.as_str()?.to_string()
    };
    let candidate = match role {
        "system" => Message::System {
            content: text.clone(),
        },
        "user" => Message::user(text.clone()),
        "assistant" => Message::assistant(text.clone()),
        _ => return None,
    };
    (serde_json::to_value(candidate).ok()?.as_object()? == raw.as_object()?).then_some(text)
}

fn normalize_message(mut message: Message) -> Message {
    match &mut message {
        Message::User { content } => content.iter_mut().for_each(normalize_user_content),
        Message::Assistant { content, .. } => {
            content.iter_mut().for_each(normalize_assistant_content)
        }
        Message::System { .. } => {}
    }
    message
}

fn normalize_user_content(content: &mut rig::message::UserContent) {
    use rig::message::UserContent;

    match content {
        UserContent::Text(text) => normalize_optional_metadata(&mut text.additional_params),
        UserContent::ToolResult(result) => result
            .content
            .iter_mut()
            .for_each(normalize_tool_result_content),
        UserContent::Image(image) => normalize_optional_metadata(&mut image.additional_params),
        UserContent::Audio(audio) => normalize_optional_metadata(&mut audio.additional_params),
        UserContent::Video(video) => normalize_optional_metadata(&mut video.additional_params),
        UserContent::Document(document) => {
            normalize_optional_metadata(&mut document.additional_params)
        }
    }
}

fn normalize_assistant_content(content: &mut rig::message::AssistantContent) {
    use rig::message::AssistantContent;

    match content {
        AssistantContent::Text(text) => normalize_optional_metadata(&mut text.additional_params),
        AssistantContent::ToolCall(call) => {
            normalize_optional_metadata(&mut call.additional_params)
        }
        AssistantContent::Reasoning(_) => {}
        AssistantContent::Image(image) => normalize_optional_metadata(&mut image.additional_params),
    }
}

fn normalize_tool_result_content(content: &mut rig::message::ToolResultContent) {
    if let rig::message::ToolResultContent::Text(text) = content {
        normalize_optional_metadata(&mut text.additional_params);
    }
}

fn normalize_optional_metadata(metadata: &mut Option<serde_json::Value>) {
    if metadata
        .as_ref()
        .is_some_and(|value| value.as_object().is_some_and(serde_json::Map::is_empty))
    {
        *metadata = None;
    }
}

#[cfg(test)]
mod tests {
    use rig::{
        OneOrMany,
        message::{
            AssistantContent, Message, ToolCall, ToolFunction, ToolResult, ToolResultContent,
            UserContent,
        },
    };

    use super::{from_provider, to_provider};

    #[test]
    fn rich_legacy_fixture_survives_domain_provider_and_persisted_json() -> anyhow::Result<()> {
        let legacy = include_str!("fixtures/rich_legacy_history.json");
        let messages: Vec<Message> = serde_json::from_str(legacy)?;
        let domain = from_provider(&messages)?;
        let restored = to_provider(&domain)?;
        let persisted = serde_json::to_value(restored)?;
        let golden: serde_json::Value = serde_json::from_str(legacy)?;
        assert_eq!(persisted, golden);
        Ok(())
    }

    #[test]
    fn simple_provider_messages_use_canonical_domain_variants() -> anyhow::Result<()> {
        use crate::entities::conversation::conversation_message::ConversationMessage;

        let messages = [
            Message::System {
                content: "rules".to_string(),
            },
            Message::user("question"),
            Message::assistant("answer"),
        ];
        let domain = from_provider(&messages)?;
        assert_eq!(
            domain,
            [
                ConversationMessage::System("rules".to_string()),
                ConversationMessage::User("question".to_string()),
                ConversationMessage::Assistant("answer".to_string()),
            ]
        );
        assert_eq!(
            serde_json::to_value(to_provider(&domain)?)?,
            serde_json::to_value(messages)?
        );
        Ok(())
    }

    #[test]
    fn empty_metadata_and_empty_reasoning_id_keep_provider_json_equal() -> anyhow::Result<()> {
        use crate::entities::conversation::conversation_message::ConversationMessage;

        let originals = [
            serde_json::json!({"role":"assistant","id":null,"content":[{"text":"plain"}]}),
            serde_json::json!({"role":"assistant","id":null,"content":[{"id":"","content":[{"type":"text","content":{"text":"thought"}}]}]}),
        ];
        let domain = originals
            .iter()
            .cloned()
            .map(|raw| ConversationMessage::Provider { raw })
            .collect::<Vec<_>>();
        let restored = to_provider(&domain)?;
        assert_eq!(
            serde_json::to_value(restored)?,
            serde_json::Value::Array(originals.into())
        );
        Ok(())
    }

    #[test]
    fn malformed_provider_message_returns_an_error() {
        let invalid = [
            crate::entities::conversation::conversation_message::ConversationMessage::Provider {
                raw: serde_json::json!({"role": "assistant", "content": "not an array"}),
            },
        ];
        assert!(to_provider(&invalid).is_err());
    }

    #[test]
    fn mixed_message_round_trip_preserves_boundaries_ids_and_metadata() -> anyhow::Result<()> {
        let source = vec![
            Message::User {
                content: OneOrMany::many(vec![
                    UserContent::Text(rig::completion::message::Text::new("question")),
                    UserContent::Image(rig::completion::message::Image {
                        data: rig::completion::message::DocumentSourceKind::Base64(
                            "png".to_string(),
                        ),
                        media_type: None,
                        detail: None,
                        additional_params: Some(serde_json::json!({"provider": "x"})),
                    }),
                    UserContent::ToolResult(ToolResult {
                        id: "result".to_string(),
                        call_id: Some("call".to_string()),
                        content: OneOrMany::many(vec![
                            ToolResultContent::text("literal"),
                            ToolResultContent::Json {
                                value: serde_json::json!({"ok": true}),
                            },
                        ])?,
                    }),
                ])?,
            },
            Message::Assistant {
                id: Some("assistant-1".to_string()),
                content: OneOrMany::many(vec![
                    AssistantContent::Text(rig::completion::message::Text {
                        text: "answer".to_string(),
                        additional_params: Some(serde_json::json!({"trace": "opaque"})),
                    }),
                    AssistantContent::ToolCall(ToolCall {
                        id: "tool-1".to_string(),
                        call_id: Some("call-1".to_string()),
                        function: ToolFunction::new(
                            "run_command".to_string(),
                            serde_json::json!({"command": "pwd"}),
                        ),
                        signature: Some("signature".to_string()),
                        additional_params: Some(serde_json::json!({"vendor": "opaque"})),
                    }),
                ])?,
            },
        ];

        let restored = to_provider(&from_provider(&source)?)?;
        assert_eq!(
            serde_json::to_value(restored)?,
            serde_json::to_value(source)?
        );
        Ok(())
    }
}
