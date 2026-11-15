use serde::{Deserialize, Serialize};

use crate::entities::{
    agent_command::queue::QueuedPrompt,
    conversation::{context_usage::ContextUsage, turn::CompactionKind},
    selected_model::selection::SelectedModel,
    transcript::entry::TranscriptEntry,
};

use super::{
    payload::{PermissionRequestView, QuestionView},
    status::{AgentActivity, GoalView, McpServerStatus, Notice, ReadOnlyState},
};

/// Immutable facts and interaction requests published by the agent side.
///
/// Durable transcript entries are authoritative; deltas only drive live rendering and never
/// create durable history on their own.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) enum AgentEvent {
    /// The durable history finished loading before the first interactive turn.
    HistoryLoaded {
        /// Ordered transcript entries from archives and the active segment.
        entries: Vec<TranscriptEntry>,
        /// Number of unreadable JSONL lines skipped while loading.
        skipped_lines: usize,
    },
    /// A durable transcript entry became part of the conversation.
    TranscriptEntryAdded(TranscriptEntry),
    /// A durable transcript entry was updated in place, such as a completed tool call.
    TranscriptEntryReplaced(TranscriptEntry),
    /// A live assistant answer delta.
    AnswerDelta {
        /// Turn identity used to discard late deltas.
        turn_id: u64,
        /// Streamed text fragment.
        text: String,
    },
    /// A live reasoning delta rendered as a separate collapsed block.
    ReasoningDelta {
        /// Turn identity used to discard late deltas.
        turn_id: u64,
        /// Streamed reasoning fragment.
        text: String,
    },
    /// A delegated child session started inside a tool block.
    SubagentStarted {
        /// Run identity assigned by the subagent runner.
        run_id: u64,
        /// Complete delegated task text; the collapsed header truncates it for display.
        prompt: String,
    },
    /// Live child progress for an expanded subagent block.
    SubagentActivity {
        /// Run identity assigned when the child session started.
        run_id: u64,
        /// Child answer, reasoning, or tool activity appended to the block.
        text: String,
    },
    /// A delegated child session finished.
    SubagentFinished {
        /// Run identity assigned when the child session started.
        run_id: u64,
        /// Final report or actionable failure text returned to the parent agent.
        report: String,
    },
    /// The agent needs an explicit tool permission decision.
    PermissionRequested(PermissionRequestView),
    /// A permission request was resolved or withdrawn by the agent.
    PermissionResolved {
        /// Correlation identifier of the resolved request.
        correlation_id: u64,
    },
    /// The agent needs a free-form answer from the user.
    QuestionAsked(QuestionView),
    /// A question was answered or withdrawn.
    QuestionResolved {
        /// Correlation identifier of the resolved question.
        correlation_id: u64,
    },
    /// The high-level agent activity changed.
    ActivityChanged(AgentActivity),
    /// Provider-reported context usage changed.
    ContextUsageChanged(ContextUsage),
    /// Context compaction started.
    CompactionStarted {
        /// Whether the user or the threshold triggered compaction.
        kind: CompactionKind,
    },
    /// Context compaction completed; `compacted` is false when the summary was not committed.
    CompactionFinished {
        /// Whether the user or the threshold triggered compaction.
        kind: CompactionKind,
        /// Whether a new summary became the active model context.
        compacted: bool,
    },
    /// Context compaction failed and the run must stop.
    CompactionFailed {
        /// Whether the user or the threshold triggered compaction.
        kind: CompactionKind,
        /// Actionable failure description.
        error: String,
    },
    /// The session model changed after a successful replacement.
    ModelChanged(SelectedModel),
    /// MCP server states changed.
    McpStatusChanged(Vec<McpServerStatus>),
    /// Queued prompts changed.
    QueueChanged(Vec<QueuedPrompt>),
    /// Goal lifecycle changed.
    GoalChanged(GoalView),
    /// A transient notice that must not enter durable history.
    Notice(Notice),
    /// Durable writes became available or were blocked.
    ReadOnlyChanged(ReadOnlyState),
    /// A soft stop finished and the process may exit.
    ShutdownComplete,
}

#[cfg(test)]
mod tests {
    use crate::entities::{
        agent_command::queue::QueuedPrompt,
        agent_event::{
            payload::{InteractionOrigin, PermissionRequestView},
            status::{AgentActivity, Notice, NoticeLevel},
        },
        conversation::{context_usage::ContextUsage, turn::CompactionKind},
        selected_model::selection::SelectedModel,
        transcript::{
            boundary::CompactionBoundary,
            entry::{EntryId, TranscriptEntry, TranscriptEntryKind},
        },
    };

    use super::AgentEvent;

    fn round_trip(event: &AgentEvent) -> anyhow::Result<AgentEvent> {
        let json = serde_json::to_value(event)?;
        Ok(serde_json::from_value(json)?)
    }

    #[test]
    fn history_loaded_preserves_entry_order_and_boundaries() -> anyhow::Result<()> {
        let event = AgentEvent::HistoryLoaded {
            entries: vec![
                TranscriptEntry::new(
                    EntryId::new(1),
                    TranscriptEntryKind::User {
                        text: "hello".to_string(),
                    },
                ),
                TranscriptEntry::new(
                    EntryId::new(2),
                    TranscriptEntryKind::Boundary(CompactionBoundary::new("chat.0.jsonl")),
                ),
            ],
            skipped_lines: 2,
        };

        assert_eq!(round_trip(&event)?, event);
        Ok(())
    }

    #[test]
    fn permission_request_keeps_its_correlation_and_origin() -> anyhow::Result<()> {
        let event = AgentEvent::PermissionRequested(PermissionRequestView::new(
            11,
            "edit_file".to_string(),
            serde_json::json!({"path": "src/main.rs"}),
            InteractionOrigin::Subagent,
        ));

        assert_eq!(round_trip(&event)?, event);
        Ok(())
    }

    #[test]
    fn status_events_keep_domain_values() -> anyhow::Result<()> {
        let events = [
            AgentEvent::ActivityChanged(AgentActivity::Stopping),
            AgentEvent::ContextUsageChanged(ContextUsage::new(Some(800), Some(1_000))),
            AgentEvent::CompactionStarted {
                kind: CompactionKind::Automatic,
            },
            AgentEvent::ModelChanged(SelectedModel::new("model-a".to_string(), Some(32_768))),
            AgentEvent::Notice(Notice::new(NoticeLevel::Warning, "skipped lines")),
        ];

        for event in events {
            assert_eq!(round_trip(&event)?, event);
        }
        Ok(())
    }

    #[test]
    fn queue_event_keeps_fifo_order() -> anyhow::Result<()> {
        let event = AgentEvent::QueueChanged(vec![
            QueuedPrompt::new(1, "first".to_string()),
            QueuedPrompt::new(2, "second".to_string()),
        ]);

        assert_eq!(round_trip(&event)?, event);
        Ok(())
    }
}
