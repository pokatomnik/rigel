use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    controllers::model_runtime::history::message_codec::{from_provider, to_provider},
    entities::conversation::conversation_message::ConversationMessage,
};

/// Version of the JSONL record schema written by this build.
pub(crate) const SCHEMA_VERSION: u32 = 1;

/// Stable identifier of one JSONL record within a history family.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) struct RecordId(u64);

impl RecordId {
    /// Creates an identifier from a persisted numeric value.
    pub(crate) const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the next identifier without wrapping on overflow.
    pub(crate) const fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

/// Payload category stored as one JSON object per line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RecordKind {
    /// One provider message.
    Message,
    /// One compaction summary stored as the active assistant message.
    Summary,
    /// A complete replacement snapshot for canonical updates.
    Snapshot,
}

/// One append-only JSONL record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct ChatLine {
    /// Schema version used to reject incompatible files.
    pub(crate) v: u32,
    /// Record category.
    pub(crate) kind: RecordKind,
    /// Stable record identifier assigned by the writer.
    pub(crate) id: RecordId,
    /// Provider message payload for `message` records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) message: Option<Value>,
    /// Summary text for `summary` records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) text: Option<String>,
    /// Complete replacement payload for `snapshot` records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) messages: Option<Vec<Value>>,
}

impl ChatLine {
    /// Creates one message record from a provider-neutral conversation message.
    pub(crate) fn message(id: RecordId, message: &ConversationMessage) -> anyhow::Result<Self> {
        Ok(Self::for_kind(id, RecordKind::Message).with_message(message_value(message)?))
    }

    /// Creates one summary record from the exact summarizer output.
    pub(crate) fn summary(id: RecordId, text: impl Into<String>) -> Self {
        let mut line = Self::for_kind(id, RecordKind::Summary);
        line.text = Some(text.into());
        line
    }

    /// Creates one replacement snapshot for canonical history updates.
    pub(crate) fn snapshot(id: RecordId, messages: &[ConversationMessage]) -> anyhow::Result<Self> {
        Ok(Self::for_kind(id, RecordKind::Snapshot).with_messages(messages_value(messages)?))
    }

    /// Reports whether the payload required by the record kind is present.
    pub(crate) fn has_payload(&self) -> bool {
        match self.kind {
            RecordKind::Message => self.message.is_some(),
            RecordKind::Summary => self.text.is_some(),
            RecordKind::Snapshot => self.messages.is_some(),
        }
    }

    /// Converts this record into the messages it contributes to the model context.
    ///
    /// `message` and `summary` contribute one message; `snapshot` replaces the accumulated list.
    pub(crate) fn to_conversation_messages(&self) -> anyhow::Result<Vec<ConversationMessage>> {
        match self.kind {
            RecordKind::Message => {
                let message = self.required_message()?;
                Ok(vec![message_from_value(message)?])
            }
            RecordKind::Summary => Ok(vec![ConversationMessage::Assistant(
                self.required_text()?.to_string(),
            )]),
            RecordKind::Snapshot => self
                .required_messages()?
                .iter()
                .map(message_from_value)
                .collect(),
        }
    }

    fn for_kind(id: RecordId, kind: RecordKind) -> Self {
        Self {
            v: SCHEMA_VERSION,
            kind,
            id,
            message: None,
            text: None,
            messages: None,
        }
    }

    fn with_message(mut self, message: Value) -> Self {
        self.message = Some(message);
        self
    }

    fn with_messages(mut self, messages: Vec<Value>) -> Self {
        self.messages = Some(messages);
        self
    }

    fn required_message(&self) -> anyhow::Result<&Value> {
        self.message
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("message record has no message payload"))
    }

    fn required_text(&self) -> anyhow::Result<&str> {
        self.text
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("summary record has no text payload"))
    }

    fn required_messages(&self) -> anyhow::Result<&[Value]> {
        self.messages
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("snapshot record has no messages payload"))
    }
}

fn message_value(message: &ConversationMessage) -> anyhow::Result<Value> {
    let provider = to_provider(std::slice::from_ref(message))?;
    let message = provider
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("conversation message produced no provider message"))?;
    Ok(serde_json::to_value(message)?)
}

fn messages_value(messages: &[ConversationMessage]) -> anyhow::Result<Vec<Value>> {
    to_provider(messages)?
        .into_iter()
        .map(|message| Ok(serde_json::to_value(message)?))
        .collect()
}

fn message_from_value(value: &Value) -> anyhow::Result<ConversationMessage> {
    let provider = serde_json::from_value(value.clone())?;
    let messages = from_provider(std::slice::from_ref(&provider))?;
    messages
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("provider message produced no conversation message"))
}

#[cfg(test)]
mod tests {
    use super::{ChatLine, RecordId, RecordKind, SCHEMA_VERSION};
    use crate::entities::conversation::conversation_message::ConversationMessage;

    #[test]
    fn message_lines_round_trip_through_json() -> anyhow::Result<()> {
        let line = ChatLine::message(
            RecordId::new(1),
            &ConversationMessage::User("hello".to_string()),
        )?;
        let json = serde_json::to_string(&line)?;
        let restored: ChatLine = serde_json::from_str(json.as_str())?;

        assert_eq!(restored, line);
        assert_eq!(restored.v, SCHEMA_VERSION);
        assert_eq!(restored.kind, RecordKind::Message);
        assert!(restored.has_payload());
        Ok(())
    }

    #[test]
    fn summary_lines_convert_to_assistant_messages() -> anyhow::Result<()> {
        let line = ChatLine::summary(RecordId::new(2), "summary");
        let messages = line.to_conversation_messages()?;

        assert_eq!(
            messages,
            [ConversationMessage::Assistant("summary".to_string())]
        );
        Ok(())
    }

    #[test]
    fn snapshot_lines_replace_the_accumulated_messages() -> anyhow::Result<()> {
        let line = ChatLine::snapshot(
            RecordId::new(3),
            &[
                ConversationMessage::User("first".to_string()),
                ConversationMessage::Assistant("second".to_string()),
            ],
        )?;
        let messages = line.to_conversation_messages()?;

        assert_eq!(
            messages,
            [
                ConversationMessage::User("first".to_string()),
                ConversationMessage::Assistant("second".to_string()),
            ]
        );
        Ok(())
    }

    #[test]
    fn missing_required_payload_is_detectable() {
        let mut line = ChatLine::summary(RecordId::new(4), "summary");
        line.text = None;

        assert!(!line.has_payload());
        assert!(line.to_conversation_messages().is_err());
    }

    #[test]
    fn record_ids_advance_without_wrapping() {
        assert_eq!(RecordId::new(1).next(), RecordId::new(2));
        assert_eq!(RecordId::new(u64::MAX).next(), RecordId::new(u64::MAX));
    }
}
