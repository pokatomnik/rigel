use crate::entities::{
    palette::item::CopyTarget,
    transcript::{
        block::SUBAGENT_TOOL_NAME,
        entry::{EntryId, TranscriptEntry, TranscriptEntryKind},
    },
};

use super::UiState;

impl UiState {
    /// Returns the complete payload for one palette copy action without truncation.
    pub(crate) fn copy_payload(&self, target: CopyTarget) -> Option<String> {
        match target {
            CopyTarget::Answer => self.last_answer(),
            CopyTarget::Summary => self.last_summary(),
            CopyTarget::Block => self.last_block(),
        }
    }

    fn last_answer(&self) -> Option<String> {
        if !self.live_answer.is_empty() {
            return Some(self.live_answer.clone());
        }
        self.entries
            .iter()
            .rev()
            .find_map(|entry| match &entry.kind {
                TranscriptEntryKind::Assistant { text, .. } => Some(text.clone()),
                _ => None,
            })
    }

    fn last_summary(&self) -> Option<String> {
        self.entries
            .iter()
            .rev()
            .find_map(|entry| match &entry.kind {
                TranscriptEntryKind::Summary { text } => Some(text.clone()),
                _ => None,
            })
    }

    fn last_block(&self) -> Option<String> {
        if !self.live_reasoning.is_empty() {
            return Some(self.live_reasoning.clone());
        }
        self.entries
            .iter()
            .rev()
            .find_map(|entry| self.block_payload(entry))
    }

    fn block_payload(&self, entry: &TranscriptEntry) -> Option<String> {
        match &entry.kind {
            TranscriptEntryKind::ToolCall {
                call_id,
                tool_name,
                arguments,
            } => Some(self.tool_payload(entry.id, tool_name, call_id, arguments)),
            TranscriptEntryKind::ToolResult {
                call_id, output, ..
            } => (!self.has_tool_call(call_id)).then(|| output.clone()),
            TranscriptEntryKind::Summary { text } => Some(text.clone()),
            TranscriptEntryKind::Assistant {
                reasoning: Some(text),
                ..
            } => Some(text.clone()),
            _ => None,
        }
    }

    fn tool_payload(
        &self,
        entry_id: EntryId,
        tool_name: &str,
        call_id: &str,
        arguments: &serde_json::Value,
    ) -> String {
        if tool_name == SUBAGENT_TOOL_NAME
            && let Some(run) = self.subagent_runs.get(&entry_id)
        {
            return format!(
                "Task:\n{}\n\nWork:\n{}\n\nReport:\n{}",
                run.prompt,
                run.activity,
                run.report.as_deref().unwrap_or("")
            );
        }
        let mut payload =
            serde_json::to_string_pretty(arguments).unwrap_or_else(|_| arguments.to_string());
        if let Some(result) = self.tool_result(call_id) {
            payload.push_str("\n\nResult:\n");
            payload.push_str(&result);
        }
        payload
    }

    fn has_tool_call(&self, call_id: &str) -> bool {
        self.entries.iter().any(|entry| {
            matches!(
                &entry.kind,
                TranscriptEntryKind::ToolCall { call_id: id, .. } if id == call_id
            )
        })
    }

    fn tool_result(&self, call_id: &str) -> Option<String> {
        self.entries.iter().find_map(|entry| match &entry.kind {
            TranscriptEntryKind::ToolResult {
                call_id: result_id,
                output,
                ..
            } if result_id == call_id => Some(output.clone()),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::UiState;
    use crate::entities::{
        application::input_event::TerminalSize,
        palette::item::CopyTarget,
        transcript::{
            block::SUBAGENT_TOOL_NAME,
            entry::{EntryId, TranscriptEntry, TranscriptEntryKind},
        },
    };

    fn state_with_assistant(text: &str) -> UiState {
        let mut state = UiState::new(TerminalSize::new(80, 24));
        state.entries.push(TranscriptEntry::new(
            EntryId::new(1),
            TranscriptEntryKind::Assistant {
                text: text.to_string(),
                reasoning: None,
            },
        ));
        state
    }

    #[test]
    fn answer_copy_takes_the_last_assistant_text() {
        let state = state_with_assistant("answer");

        assert_eq!(
            state.copy_payload(CopyTarget::Answer),
            Some("answer".to_string())
        );
    }

    #[test]
    fn tool_copy_keeps_arguments_and_full_result() -> anyhow::Result<()> {
        let mut state = UiState::new(TerminalSize::new(80, 24));
        let id = EntryId::new(2);
        state.entries.push(TranscriptEntry::new(
            id,
            TranscriptEntryKind::ToolCall {
                call_id: "call-1".to_string(),
                tool_name: "read_file".to_string(),
                arguments: serde_json::json!({"path": "src/main.rs"}),
            },
        ));
        state.entries.push(TranscriptEntry::new(
            EntryId::new(3),
            TranscriptEntryKind::ToolResult {
                call_id: "call-1".to_string(),
                tool_name: "read_file".to_string(),
                output: "complete output".to_string(),
                failed: false,
            },
        ));
        let payload = state
            .copy_payload(CopyTarget::Block)
            .ok_or_else(|| anyhow::anyhow!("missing payload"))?;

        assert!(payload.contains("\"src/main.rs\""));
        assert!(payload.contains("complete output"));
        Ok(())
    }

    #[test]
    fn subagent_copy_contains_prompt_work_and_report() {
        let mut state = UiState::new(TerminalSize::new(80, 24));
        let id = EntryId::new(4);
        state.entries.push(TranscriptEntry::new(
            id,
            TranscriptEntryKind::ToolCall {
                call_id: "call-2".to_string(),
                tool_name: SUBAGENT_TOOL_NAME.to_string(),
                arguments: serde_json::json!({"task": "inspect"}),
            },
        ));
        state.subagent_started(1, "inspect".to_string());
        state.subagent_activity(1, "work");
        state.subagent_finished(1, "report".to_string());

        let payload = state.copy_payload(CopyTarget::Block).unwrap_or_default();
        assert!(payload.contains("inspect") && payload.contains("work"));
        assert!(payload.contains("report"));
    }

    #[test]
    fn missing_payload_reports_nothing_to_copy() {
        let state = UiState::new(TerminalSize::new(80, 24));

        assert_eq!(state.copy_payload(CopyTarget::Answer), None);
    }
}
