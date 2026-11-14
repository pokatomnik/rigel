use crate::{
    controllers::storage::record::{ChatLine, RecordKind},
    entities::{
        conversation::conversation_message::ConversationMessage,
        transcript::{
            boundary::CompactionBoundary,
            entry::{EntryId, TranscriptEntry, TranscriptEntryKind},
        },
    },
};

/// Projects durable JSONL lines into the transcript shown after startup.
///
/// Archive boundaries are retained so the UI can explain where model-invisible history went.
/// Snapshot records are skipped because they repeat messages already present as ordinary
/// records. Provider tool calls and results are restored so blocks keep their full payload.
pub(crate) fn project_history(
    archives: &[(String, Vec<ChatLine>)],
    active: &[ChatLine],
) -> Vec<TranscriptEntry> {
    let mut projector = Projector { next_id: 1 };
    let mut entries = Vec::new();
    for (file_name, lines) in archives {
        entries.push(
            projector.entry(TranscriptEntryKind::Boundary(CompactionBoundary::new(
                file_name.clone(),
            ))),
        );
        projector.project_lines(lines, &mut entries);
    }
    projector.project_lines(active, &mut entries);
    entries
}

struct Projector {
    next_id: u64,
}

impl Projector {
    fn unit(&mut self) -> EntryId {
        let id = EntryId::new(self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    fn entry(&mut self, kind: TranscriptEntryKind) -> TranscriptEntry {
        TranscriptEntry::new(self.unit(), kind)
    }

    fn project_lines(&mut self, lines: &[ChatLine], entries: &mut Vec<TranscriptEntry>) {
        for line in lines {
            if line.kind == RecordKind::Snapshot {
                continue;
            }
            if line.kind == RecordKind::Summary {
                if let Some(text) = line.text.clone() {
                    entries.push(self.entry(TranscriptEntryKind::Summary { text }));
                }
                continue;
            }
            let Ok(messages) = line.to_conversation_messages() else {
                continue;
            };
            for message in messages {
                self.project_message(message, entries);
            }
        }
    }

    fn project_message(
        &mut self,
        message: ConversationMessage,
        entries: &mut Vec<TranscriptEntry>,
    ) {
        match message {
            ConversationMessage::System(_) => {}
            ConversationMessage::User(text) => {
                entries.push(self.entry(TranscriptEntryKind::User { text }));
            }
            ConversationMessage::Assistant(text) => {
                entries.push(self.entry(TranscriptEntryKind::Assistant {
                    text,
                    reasoning: None,
                }));
            }
            ConversationMessage::Provider { raw } => self.project_provider(&raw, entries),
        }
    }

    fn project_provider(&mut self, raw: &serde_json::Value, entries: &mut Vec<TranscriptEntry>) {
        let role = raw.get("role").and_then(serde_json::Value::as_str);
        let Some(content) = raw.get("content").and_then(serde_json::Value::as_array) else {
            return;
        };
        match role {
            Some("user") => self.project_user_parts(content, entries),
            Some("assistant") => self.project_assistant_parts(content, entries),
            _ => {}
        }
    }

    fn project_user_parts(
        &mut self,
        parts: &[serde_json::Value],
        entries: &mut Vec<TranscriptEntry>,
    ) {
        let mut text = Vec::new();
        for part in parts {
            match part.get("type").and_then(serde_json::Value::as_str) {
                Some("text") => push_text_part(part, &mut text),
                Some("toolresult") => {
                    self.flush_user_text(&mut text, entries);
                    self.push_tool_result(part, entries);
                }
                _ => {}
            }
        }
        self.flush_user_text(&mut text, entries);
    }

    fn flush_user_text(&mut self, text: &mut Vec<String>, entries: &mut Vec<TranscriptEntry>) {
        if text.is_empty() {
            return;
        }
        let joined = text.join("\n");
        text.clear();
        entries.push(self.entry(TranscriptEntryKind::User { text: joined }));
    }

    fn project_assistant_parts(
        &mut self,
        parts: &[serde_json::Value],
        entries: &mut Vec<TranscriptEntry>,
    ) {
        let mut reasoning = String::new();
        for part in parts {
            if let Some(function) = part.get("function") {
                self.push_tool_call(part, function, entries);
                continue;
            }
            if let Some(text) = part.get("text").and_then(serde_json::Value::as_str) {
                entries.push(self.entry(TranscriptEntryKind::Assistant {
                    text: text.to_string(),
                    reasoning: take_text(&mut reasoning),
                }));
                continue;
            }
            push_reasoning_part(part, &mut reasoning);
        }
        if !reasoning.is_empty() {
            entries.push(self.entry(TranscriptEntryKind::Assistant {
                text: String::new(),
                reasoning: Some(reasoning),
            }));
        }
    }

    fn push_tool_call(
        &mut self,
        part: &serde_json::Value,
        function: &serde_json::Value,
        entries: &mut Vec<TranscriptEntry>,
    ) {
        let Some(tool_name) = function.get("name").and_then(serde_json::Value::as_str) else {
            return;
        };
        let Some(call_id) = call_id(part) else {
            return;
        };
        let arguments = function
            .get("arguments")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        entries.push(TranscriptEntry::new(
            EntryId::from_call_id(call_id.as_str()),
            TranscriptEntryKind::ToolCall {
                call_id,
                tool_name: tool_name.to_string(),
                arguments,
            },
        ));
    }

    fn push_tool_result(&mut self, part: &serde_json::Value, entries: &mut Vec<TranscriptEntry>) {
        let Some(call_id) = call_id(part) else {
            return;
        };
        let tool_name = entries
            .iter()
            .rev()
            .find_map(|entry| match &entry.kind {
                TranscriptEntryKind::ToolCall {
                    call_id: id,
                    tool_name,
                    ..
                } if id == &call_id => Some(tool_name.clone()),
                _ => None,
            })
            .unwrap_or_else(|| "tool".to_string());
        let (output, failed) = tool_result_text(part);
        entries.push(TranscriptEntry::new(
            EntryId::from_call_id(call_id.as_str()),
            TranscriptEntryKind::ToolResult {
                call_id,
                tool_name,
                output,
                failed,
            },
        ));
    }
}

fn call_id(part: &serde_json::Value) -> Option<String> {
    part.get("call_id")
        .or_else(|| part.get("id"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

fn push_text_part(part: &serde_json::Value, text: &mut Vec<String>) {
    if let Some(value) = part.get("text").and_then(serde_json::Value::as_str) {
        text.push(value.to_string());
    }
}

fn push_reasoning_part(part: &serde_json::Value, reasoning: &mut String) {
    let Some(parts) = part.get("content").and_then(serde_json::Value::as_array) else {
        return;
    };
    for item in parts {
        let text = item
            .get("content")
            .and_then(|content| content.get("text"))
            .or_else(|| item.get("text"))
            .and_then(serde_json::Value::as_str);
        if let Some(text) = text {
            reasoning.push_str(text);
        }
    }
}

fn take_text(text: &mut String) -> Option<String> {
    (!text.is_empty()).then(|| std::mem::take(text))
}

fn tool_result_text(part: &serde_json::Value) -> (String, bool) {
    let Some(parts) = part.get("content").and_then(serde_json::Value::as_array) else {
        return (String::new(), false);
    };
    let mut failed = false;
    let output = parts
        .iter()
        .filter_map(
            |item| match item.get("type").and_then(serde_json::Value::as_str) {
                Some("text") => item
                    .get("text")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
                Some("json") => {
                    let value = item.get("value")?;
                    if value
                        .get("rigel_tool_status")
                        .and_then(serde_json::Value::as_str)
                        == Some("error")
                    {
                        failed = true;
                        return None;
                    }
                    Some(serde_json::to_string_pretty(value).unwrap_or_default())
                }
                _ => None,
            },
        )
        .collect::<Vec<_>>()
        .join("\n");
    (output, failed)
}

#[cfg(test)]
mod tests {
    use super::project_history;
    use crate::{
        controllers::storage::record::{ChatLine, RecordId},
        entities::{
            conversation::conversation_message::ConversationMessage,
            transcript::entry::{EntryId, TranscriptEntryKind},
        },
    };

    fn provider_line(id: u64, raw: serde_json::Value) -> anyhow::Result<ChatLine> {
        ChatLine::message(RecordId::new(id), &ConversationMessage::Provider { raw })
    }

    #[test]
    fn archives_add_a_boundary_before_their_entries() -> anyhow::Result<()> {
        let archive = vec![ChatLine::message(
            RecordId::new(1),
            &ConversationMessage::User("archived".to_string()),
        )?];
        let active = vec![ChatLine::summary(RecordId::new(2), "summary")];
        let entries = project_history(&[("chat.0.jsonl".to_string(), archive)], &active);

        assert!(matches!(
            entries.first().map(|entry| &entry.kind),
            Some(TranscriptEntryKind::Boundary(boundary)) if boundary.archive == "chat.0.jsonl"
        ));
        assert!(entries.iter().any(|entry| matches!(
            &entry.kind,
            TranscriptEntryKind::Summary { text } if text == "summary"
        )));
        assert_eq!(entries.first().map(|entry| entry.id), Some(EntryId::new(1)));
        Ok(())
    }

    #[test]
    fn snapshots_do_not_duplicate_visible_history() -> anyhow::Result<()> {
        let snapshot = ChatLine::snapshot(
            RecordId::new(2),
            &[ConversationMessage::User("first".to_string())],
        )?;
        let entries = project_history(&[], &[snapshot]);

        assert!(entries.is_empty());
        Ok(())
    }

    #[test]
    fn provider_messages_keep_their_role_text() -> anyhow::Result<()> {
        let raw = serde_json::json!({
            "role": "assistant",
            "content": [{"type": "text", "text": "answer"}]
        });
        let entries = project_history(&[], &[provider_line(1, raw)?]);

        assert!(matches!(
            entries.first().map(|entry| &entry.kind),
            Some(TranscriptEntryKind::Assistant { text, .. }) if text == "answer"
        ));
        Ok(())
    }

    #[test]
    fn tool_calls_and_results_are_restored_with_full_payloads() -> anyhow::Result<()> {
        let assistant = serde_json::json!({
            "role": "assistant",
            "content": [{
                "id": "tool-1",
                "call_id": "call-1",
                "function": {"name": "read_file", "arguments": {"path": "src/main.rs"}}
            }]
        });
        let user = serde_json::json!({
            "role": "user",
            "content": [{
                "type": "toolresult",
                "id": "result-1",
                "call_id": "call-1",
                "content": [{"type": "text", "text": "contents"}]
            }]
        });
        let entries = project_history(
            &[],
            &[provider_line(1, assistant)?, provider_line(2, user)?],
        );

        assert!(matches!(
            entries.first().map(|entry| &entry.kind),
            Some(TranscriptEntryKind::ToolCall { tool_name, arguments, .. })
                if tool_name == "read_file" && arguments["path"] == "src/main.rs"
        ));
        assert_eq!(
            entries.first().map(|entry| entry.id),
            entries.get(1).map(|entry| entry.id)
        );
        assert!(matches!(
            entries.get(1).map(|entry| &entry.kind),
            Some(TranscriptEntryKind::ToolResult { tool_name, output, .. })
                if tool_name == "read_file" && output == "contents"
        ));
        Ok(())
    }

    #[test]
    fn recovery_markers_are_hidden_and_mark_the_result_failed() -> anyhow::Result<()> {
        let user = serde_json::json!({
            "role": "user",
            "content": [{
                "type": "toolresult",
                "id": "result-1",
                "call_id": "call-1",
                "content": [
                    {"type": "text", "text": "boom"},
                    {"type": "json", "value": {"rigel_tool_status": "error"}}
                ]
            }]
        });
        let entries = project_history(&[], &[provider_line(1, user)?]);

        assert!(matches!(
            entries.first().map(|entry| &entry.kind),
            Some(TranscriptEntryKind::ToolResult { output, failed, .. })
                if output == "boom" && *failed
        ));
        Ok(())
    }

    #[test]
    fn assistant_reasoning_is_restored_next_to_its_answer() -> anyhow::Result<()> {
        let raw = serde_json::json!({
            "role": "assistant",
            "content": [
                {"id": "reason-1", "content": [{"type": "text", "content": {"text": "thought"}}]},
                {"type": "text", "text": "answer"}
            ]
        });
        let entries = project_history(&[], &[provider_line(1, raw)?]);

        assert!(matches!(
            entries.first().map(|entry| &entry.kind),
            Some(TranscriptEntryKind::Assistant { text, reasoning: Some(reasoning) })
                if text == "answer" && reasoning == "thought"
        ));
        Ok(())
    }
}
