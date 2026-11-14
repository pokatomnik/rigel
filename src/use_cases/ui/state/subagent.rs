use crate::entities::{
    agent_event::event::AgentEvent,
    render_model::transcript::SubagentRow,
    transcript::{
        block::SUBAGENT_TOOL_NAME,
        entry::{EntryId, TranscriptEntryKind},
    },
};

use super::UiState;

impl UiState {
    /// Applies one subagent lifecycle event and reports whether it was consumed.
    pub(crate) fn apply_subagent(&mut self, event: &AgentEvent) -> bool {
        match event {
            AgentEvent::SubagentStarted { run_id, prompt } => {
                self.subagent_started(*run_id, prompt.clone());
            }
            AgentEvent::SubagentActivity { run_id, text } => {
                self.subagent_activity(*run_id, text);
            }
            AgentEvent::SubagentFinished { run_id, report } => {
                self.subagent_finished(*run_id, report.clone());
            }
            _ => return false,
        }
        true
    }
    /// Links a started child run to the newest unclaimed `spawn_subagent` tool block.
    pub(crate) fn subagent_started(&mut self, run_id: u64, prompt: String) {
        let Some(entry_id) = self.unclaimed_subagent_entry() else {
            return;
        };
        self.subagent_links.insert(run_id, entry_id);
        self.subagent_runs
            .insert(entry_id, SubagentRow::new(prompt));
    }

    /// Appends child answer, reasoning, or tool activity to its tool block.
    pub(crate) fn subagent_activity(&mut self, run_id: u64, text: &str) {
        let Some(entry_id) = self.subagent_links.get(&run_id).copied() else {
            return;
        };
        if let Some(run) = self.subagent_runs.get_mut(&entry_id) {
            run.activity.push_str(text);
        }
    }

    /// Marks the child run finished and keeps its report inside the block.
    pub(crate) fn subagent_finished(&mut self, run_id: u64, report: String) {
        let Some(entry_id) = self.subagent_links.get(&run_id).copied() else {
            return;
        };
        if let Some(run) = self.subagent_runs.get_mut(&entry_id) {
            run.report = Some(report);
        }
    }

    /// Returns the isolated child work attached to one tool block.
    pub(crate) fn subagent(&self, entry_id: EntryId) -> Option<&SubagentRow> {
        self.subagent_runs.get(&entry_id)
    }

    fn unclaimed_subagent_entry(&self) -> Option<EntryId> {
        self.entries
            .iter()
            .find(|entry| {
                matches!(
                    &entry.kind,
                    TranscriptEntryKind::ToolCall { tool_name, .. }
                        if tool_name == SUBAGENT_TOOL_NAME
                ) && !self.subagent_runs.contains_key(&entry.id)
            })
            .map(|entry| entry.id)
    }
}

#[cfg(test)]
mod tests {
    use super::super::UiState;
    use crate::entities::{
        application::input_event::TerminalSize,
        transcript::{
            block::SUBAGENT_TOOL_NAME,
            entry::{EntryId, TranscriptEntry, TranscriptEntryKind},
        },
    };

    fn state_with_call(call_id: &str) -> (UiState, EntryId) {
        let mut state = UiState::new(TerminalSize::new(80, 24));
        let id = EntryId::from_call_id(call_id);
        state.entries.push(TranscriptEntry::new(
            id,
            TranscriptEntryKind::ToolCall {
                call_id: call_id.to_string(),
                tool_name: SUBAGENT_TOOL_NAME.to_string(),
                arguments: serde_json::json!({"task": "inspect"}),
            },
        ));
        (state, id)
    }

    #[test]
    fn started_activity_and_report_are_attached_to_the_spawn_block() {
        let (mut state, id) = state_with_call("call-1");
        state.subagent_started(1, "inspect the repository".to_string());
        state.subagent_activity(1, "\n[read_file]\n");
        state.subagent_finished(1, "report".to_string());

        let run = state.subagent(id);
        assert!(run.is_some_and(|run| {
            run.prompt == "inspect the repository"
                && run.activity == "\n[read_file]\n"
                && run.report.as_deref() == Some("report")
        }));
    }

    #[test]
    fn activity_for_an_unknown_run_is_ignored() {
        let (mut state, id) = state_with_call("call-2");
        state.subagent_activity(9, "orphan");

        assert!(state.subagent(id).is_none());
    }

    #[test]
    fn each_started_run_claims_a_distinct_spawn_block() {
        let (mut state, first) = state_with_call("call-3");
        let second = EntryId::from_call_id("call-4");
        state.entries.push(TranscriptEntry::new(
            second,
            TranscriptEntryKind::ToolCall {
                call_id: "call-4".to_string(),
                tool_name: SUBAGENT_TOOL_NAME.to_string(),
                arguments: serde_json::json!({"task": "other"}),
            },
        ));
        state.subagent_started(1, "first".to_string());
        state.subagent_started(2, "second".to_string());

        assert!(
            state
                .subagent(first)
                .is_some_and(|run| run.prompt == "first")
        );
        assert!(
            state
                .subagent(second)
                .is_some_and(|run| run.prompt == "second")
        );
    }
}
