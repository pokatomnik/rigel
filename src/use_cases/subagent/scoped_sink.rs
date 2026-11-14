use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

use crate::{
    entities::{
        agent_event::{
            event::AgentEvent,
            payload::{PermissionRequestView, QuestionView},
        },
        transcript::entry::{TranscriptEntry, TranscriptEntryKind},
    },
    infrastructure::ui::gateway::EventSink,
};

const MODE_PLAIN: u8 = 0;
const MODE_REASONING: u8 = 1;
const MODE_ANSWER: u8 = 2;

/// Translates child session events into parent-visible subagent activity.
///
/// Only presentation facts are forwarded. Parent session state such as activity, context usage,
/// model identity, and queue contents must never be overwritten by a child session.
pub(crate) struct ScopedSubagentSink {
    parent: Arc<dyn EventSink>,
    run_id: u64,
    mode: AtomicU8,
}

impl ScopedSubagentSink {
    /// Creates a sink that attributes every child activity event to one run.
    pub(crate) fn new(parent: Arc<dyn EventSink>, run_id: u64) -> Self {
        Self {
            parent,
            run_id,
            mode: AtomicU8::new(MODE_PLAIN),
        }
    }

    fn activity(&self, text: impl Into<String>) {
        self.parent.emit(AgentEvent::SubagentActivity {
            run_id: self.run_id,
            text: text.into(),
        });
    }

    /// Appends one presentation line to the parent subagent block.
    pub(crate) fn push_text(&self, text: impl Into<String>) {
        self.activity(text);
    }

    fn reasoning(&self, text: &str) {
        self.switch(MODE_REASONING, "thinking");
        self.activity(text);
    }

    fn answer(&self, text: &str) {
        self.switch(MODE_ANSWER, "answer");
        self.activity(text);
    }

    fn switch(&self, mode: u8, label: &str) {
        if self.mode.swap(mode, Ordering::Relaxed) != mode {
            self.activity(format!("\n{label}: "));
        }
    }

    fn tool_call(&self, tool_name: &str) {
        self.mode.store(MODE_PLAIN, Ordering::Relaxed);
        self.activity(format!("\n[{tool_name}]\n"));
    }

    fn transcript_entry(&self, entry: &TranscriptEntry) {
        if let TranscriptEntryKind::ToolCall { tool_name, .. } = &entry.kind {
            self.tool_call(tool_name);
        }
    }

    fn forward_permission(&self, request: PermissionRequestView) {
        self.parent.emit(AgentEvent::PermissionRequested(request));
    }

    fn forward_question(&self, question: QuestionView) {
        self.parent.emit(AgentEvent::QuestionAsked(question));
    }
}

impl EventSink for ScopedSubagentSink {
    fn emit(&self, event: AgentEvent) {
        match event {
            AgentEvent::AnswerDelta { text, .. } => self.answer(text.as_str()),
            AgentEvent::ReasoningDelta { text, .. } => self.reasoning(text.as_str()),
            AgentEvent::TranscriptEntryAdded(entry) => self.transcript_entry(&entry),
            AgentEvent::PermissionRequested(request) => self.forward_permission(request),
            AgentEvent::QuestionAsked(question) => self.forward_question(question),
            forwarded @ (AgentEvent::PermissionResolved { .. }
            | AgentEvent::QuestionResolved { .. }) => self.parent.emit(forwarded),
            AgentEvent::HistoryLoaded { .. }
            | AgentEvent::Notice(_)
            | AgentEvent::ReadOnlyChanged(_)
            | AgentEvent::CompactionStarted { .. }
            | AgentEvent::CompactionFinished { .. }
            | AgentEvent::CompactionFailed { .. }
            | AgentEvent::TranscriptEntryReplaced(_)
            | AgentEvent::SubagentStarted { .. }
            | AgentEvent::SubagentActivity { .. }
            | AgentEvent::SubagentFinished { .. }
            | AgentEvent::ActivityChanged(_)
            | AgentEvent::ContextUsageChanged(_)
            | AgentEvent::ModelChanged(_)
            | AgentEvent::McpStatusChanged(_)
            | AgentEvent::QueueChanged(_)
            | AgentEvent::GoalChanged(_)
            | AgentEvent::ShutdownComplete => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::ScopedSubagentSink;
    use crate::{
        entities::{
            agent_event::{
                event::AgentEvent,
                status::{AgentActivity, Notice, NoticeLevel},
            },
            transcript::entry::{EntryId, TranscriptEntry, TranscriptEntryKind},
        },
        infrastructure::ui::gateway::EventSink,
    };

    #[derive(Default)]
    struct Recorder {
        events: Mutex<Vec<AgentEvent>>,
    }

    impl EventSink for Recorder {
        fn emit(&self, event: AgentEvent) {
            if let Ok(mut events) = self.events.lock() {
                events.push(event);
            }
        }
    }

    fn sink() -> (Arc<Recorder>, ScopedSubagentSink) {
        let recorder = Arc::new(Recorder::default());
        (recorder.clone(), ScopedSubagentSink::new(recorder, 7))
    }

    fn activity_text(events: &[AgentEvent]) -> String {
        events
            .iter()
            .filter_map(|event| match event {
                AgentEvent::SubagentActivity { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn child_answer_and_reasoning_become_attributed_activity() -> anyhow::Result<()> {
        let (recorder, sink) = sink();
        sink.emit(AgentEvent::ReasoningDelta {
            turn_id: 1,
            text: "why".to_string(),
        });
        sink.emit(AgentEvent::AnswerDelta {
            turn_id: 1,
            text: "done".to_string(),
        });
        let events = recorder
            .events
            .lock()
            .map_err(|_| anyhow::anyhow!("poisoned"))?;

        assert_eq!(
            activity_text(events.as_slice()),
            "\nthinking: why\nanswer: done"
        );
        assert!(
            events
                .iter()
                .all(|event| matches!(event, AgentEvent::SubagentActivity { run_id: 7, .. }))
        );
        Ok(())
    }

    #[test]
    fn child_tool_calls_are_visible_but_results_are_not_duplicated() -> anyhow::Result<()> {
        let (recorder, sink) = sink();
        sink.emit(AgentEvent::TranscriptEntryAdded(TranscriptEntry::new(
            EntryId::new(1),
            TranscriptEntryKind::ToolCall {
                call_id: "call".to_string(),
                tool_name: "read_file".to_string(),
                arguments: serde_json::json!({}),
            },
        )));
        sink.emit(AgentEvent::TranscriptEntryReplaced(TranscriptEntry::new(
            EntryId::new(1),
            TranscriptEntryKind::ToolResult {
                call_id: "call".to_string(),
                tool_name: "read_file".to_string(),
                output: "contents".to_string(),
                failed: false,
            },
        )));
        let events = recorder
            .events
            .lock()
            .map_err(|_| anyhow::anyhow!("poisoned"))?;

        assert_eq!(activity_text(events.as_slice()), "\n[read_file]\n");
        Ok(())
    }

    #[test]
    fn parent_state_events_are_not_overwritten_by_children() -> anyhow::Result<()> {
        let (recorder, sink) = sink();
        sink.emit(AgentEvent::ActivityChanged(AgentActivity::Running));
        sink.emit(AgentEvent::ContextUsageChanged(
            crate::entities::conversation::context_usage::ContextUsage::new(Some(1), Some(2)),
        ));
        sink.emit(AgentEvent::Notice(Notice::new(
            NoticeLevel::Info,
            "visible",
        )));
        sink.emit(AgentEvent::CompactionStarted {
            kind: crate::entities::conversation::turn::CompactionKind::Automatic,
        });
        let events = recorder
            .events
            .lock()
            .map_err(|_| anyhow::anyhow!("poisoned"))?;

        assert!(events.is_empty());
        Ok(())
    }
}
