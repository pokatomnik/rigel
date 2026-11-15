use std::collections::HashMap;

use crate::entities::{
    render_model::{
        model::RenderModel,
        transcript::{ToolResultRow, ToolRow, TranscriptRow, TranscriptView},
        views::{ComposerView, HeaderView, McpSummary, StatusView},
    },
    transcript::entry::{EntryId, TranscriptEntry, TranscriptEntryKind},
};

use super::UiState;

impl UiState {
    /// Builds the immutable frame handed to the Ratatui renderer.
    pub(crate) fn render_model(&self) -> RenderModel {
        let header = HeaderView::new(self.model.clone(), self.context)
            .with_activity(self.activity)
            .with_mcp(
                McpSummary::new(
                    self.mcp
                        .iter()
                        .filter(|status| {
                            status.state
                                == crate::entities::agent_event::status::McpServerState::Running
                        })
                        .count(),
                    self.mcp.len(),
                )
                .with_starting(
                    self.mcp
                        .iter()
                        .filter(|status| {
                            status.state
                                == crate::entities::agent_event::status::McpServerState::Starting
                        })
                        .count(),
                ),
            );
        RenderModel::empty(self.size)
            .with_header(header)
            .with_transcript(self.transcript_view())
            .with_composer(ComposerView::default())
            .with_status(self.status_view())
            .with_modal(self.modals.last().cloned())
            .with_palette(self.palette.as_ref().map(|palette| palette.view()))
    }

    fn transcript_view(&self) -> TranscriptView {
        let mut rows = self.transcript_rows();
        if !self.live_reasoning.is_empty() {
            rows.push(TranscriptRow::LiveReasoning {
                text: self.live_reasoning.clone(),
                expanded: self.live_reasoning_expanded,
            });
        }
        if !self.live_answer.is_empty() {
            rows.push(TranscriptRow::LiveAssistant {
                text: self.live_answer.clone(),
            });
        }
        TranscriptView::new(rows, self.expanded.clone())
    }

    fn transcript_rows(&self) -> Vec<TranscriptRow> {
        let results = tool_results(self.entries.as_slice());
        let mut rows = Vec::with_capacity(self.entries.len());
        for entry in &self.entries {
            match &entry.kind {
                TranscriptEntryKind::User { text } => rows.push(TranscriptRow::User {
                    id: entry.id,
                    text: text.clone(),
                }),
                TranscriptEntryKind::Assistant { text, reasoning } => {
                    rows.push(TranscriptRow::Assistant {
                        id: entry.id,
                        text: text.clone(),
                        reasoning: reasoning.clone(),
                    });
                }
                TranscriptEntryKind::ToolCall {
                    call_id,
                    tool_name,
                    arguments,
                } => rows.push(tool_row(
                    entry.id,
                    tool_name,
                    arguments,
                    results.get(call_id.as_str()),
                    self.subagent(entry.id),
                )),
                TranscriptEntryKind::ToolResult {
                    call_id,
                    tool_name,
                    output,
                    failed,
                } if !results.contains_key(call_id.as_str()) => {
                    rows.push(TranscriptRow::Tool(
                        ToolRow::new(entry.id, tool_name.clone(), String::new())
                            .with_result(ToolResultRow::new(output.clone(), *failed)),
                    ));
                }
                TranscriptEntryKind::ToolResult { .. } => {}
                TranscriptEntryKind::Summary { text } => rows.push(TranscriptRow::Summary {
                    id: entry.id,
                    text: text.clone(),
                }),
                TranscriptEntryKind::Boundary(boundary) => rows.push(TranscriptRow::Boundary {
                    id: entry.id,
                    archive: boundary.archive.clone(),
                }),
            }
        }
        rows
    }

    fn status_view(&self) -> StatusView {
        let hint = if self.queue_len > 0 {
            Some(format!(
                "{} queued · Enter: send · Alt+Enter: newline",
                self.queue_len
            ))
        } else {
            goal_hint(self.goal())
        };
        StatusView::new(self.activity, self.read_only.clone())
            .with_notices(self.notices.clone())
            .with_hint(hint)
    }
}

fn goal_hint(goal: &crate::entities::agent_event::status::GoalView) -> Option<String> {
    use crate::entities::agent_event::status::GoalPhase;
    match goal.phase {
        GoalPhase::Inactive => None,
        GoalPhase::Active => Some("Goal is active · Esc to stop".to_string()),
        GoalPhase::Completed => Some("Goal completed".to_string()),
    }
}

fn tool_row(
    id: EntryId,
    tool_name: &str,
    arguments: &serde_json::Value,
    result: Option<&ToolResultRow>,
    subagent: Option<&crate::entities::render_model::transcript::SubagentRow>,
) -> TranscriptRow {
    let arguments =
        serde_json::to_string_pretty(arguments).unwrap_or_else(|_| arguments.to_string());
    let mut row = ToolRow::new(id, tool_name.to_string(), arguments);
    if let Some(result) = result {
        row = row.with_result(result.clone());
    }
    if let Some(subagent) = subagent {
        row = row.with_subagent(subagent.clone());
    }
    TranscriptRow::Tool(row)
}

fn tool_results(entries: &[TranscriptEntry]) -> HashMap<String, ToolResultRow> {
    entries
        .iter()
        .filter_map(|entry| match &entry.kind {
            TranscriptEntryKind::ToolResult {
                call_id,
                output,
                failed,
                ..
            } => Some((call_id.clone(), ToolResultRow::new(output.clone(), *failed))),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::UiState;
    use crate::entities::{
        agent_event::{
            event::AgentEvent,
            status::{AgentActivity, Notice, NoticeLevel},
        },
        application::input_event::TerminalSize,
        transcript::{
            block::SUBAGENT_TOOL_NAME,
            entry::{EntryId, TranscriptEntry, TranscriptEntryKind},
        },
    };

    fn state() -> UiState {
        UiState::new(TerminalSize::new(80, 24))
    }

    #[test]
    fn streamed_answer_is_replaced_by_the_durable_entry() {
        let mut state = state();
        state.apply(AgentEvent::AnswerDelta {
            turn_id: 1,
            text: "partial".to_string(),
        });
        state.apply(AgentEvent::TranscriptEntryAdded(TranscriptEntry::new(
            EntryId::new(1),
            TranscriptEntryKind::Assistant {
                text: "final answer".to_string(),
                reasoning: None,
            },
        )));

        let model = state.render_model();
        assert!(!format!("{:?}", model.transcript.rows).contains("partial"));
        assert!(format!("{:?}", model.transcript.rows).contains("final answer"));
    }

    #[test]
    fn tool_result_updates_the_matching_call_row() -> anyhow::Result<()> {
        let mut state = state();
        let id = EntryId::from_call_id("call-1");
        state.apply(AgentEvent::TranscriptEntryAdded(TranscriptEntry::new(
            id,
            TranscriptEntryKind::ToolCall {
                call_id: "call-1".to_string(),
                tool_name: "read_file".to_string(),
                arguments: serde_json::json!({"path": "src/main.rs"}),
            },
        )));
        state.apply(AgentEvent::TranscriptEntryReplaced(TranscriptEntry::new(
            id,
            TranscriptEntryKind::ToolResult {
                call_id: "call-1".to_string(),
                tool_name: "read_file".to_string(),
                output: "contents".to_string(),
                failed: false,
            },
        )));

        let model = state.render_model();
        let Some(crate::entities::render_model::transcript::TranscriptRow::Tool(row)) =
            model.transcript.rows.first()
        else {
            anyhow::bail!("expected a tool row");
        };
        assert_eq!(
            row.result,
            Some(
                crate::entities::render_model::transcript::ToolResultRow::new(
                    "contents".to_string(),
                    false
                )
            )
        );
        Ok(())
    }

    #[test]
    fn subagent_tool_keeps_its_collapsible_name() {
        let mut state = state();
        state.apply(AgentEvent::TranscriptEntryAdded(TranscriptEntry::new(
            EntryId::new(1),
            TranscriptEntryKind::ToolCall {
                call_id: "call-1".to_string(),
                tool_name: SUBAGENT_TOOL_NAME.to_string(),
                arguments: serde_json::json!({"task": "inspect"}),
            },
        )));

        let model = state.render_model();
        assert!(format!("{:?}", model.transcript.rows).contains(SUBAGENT_TOOL_NAME));
    }

    #[test]
    fn subagent_permission_uses_the_same_modal() {
        let mut state = state();
        state.apply(AgentEvent::PermissionRequested(
            crate::entities::agent_event::payload::PermissionRequestView::new(
                3,
                "edit_file".to_string(),
                serde_json::json!({"path": "src/main.rs"}),
                crate::entities::agent_event::payload::InteractionOrigin::Subagent,
            ),
        ));

        let model = state.render_model();
        assert!(matches!(
            model.modal,
            Some(crate::entities::render_model::modal::ModalView::Permission { request, .. })
                if request.origin
                    == crate::entities::agent_event::payload::InteractionOrigin::Subagent
        ));
    }

    #[test]
    fn live_reasoning_starts_collapsed_and_toggles_manually() {
        let mut state = state();
        state.apply(AgentEvent::ReasoningDelta {
            turn_id: 1,
            text: "thought".to_string(),
        });
        let collapsed = state.render_model();
        assert!(matches!(
            collapsed.transcript.rows.first(),
            Some(
                crate::entities::render_model::transcript::TranscriptRow::LiveReasoning {
                    expanded: false,
                    ..
                }
            )
        ));

        state.toggle_last_block();
        let expanded = state.render_model();
        assert!(matches!(
            expanded.transcript.rows.first(),
            Some(
                crate::entities::render_model::transcript::TranscriptRow::LiveReasoning {
                    expanded: true,
                    ..
                }
            )
        ));
    }

    #[test]
    fn manually_expanded_reasoning_survives_completion() {
        let mut state = state();
        let id = EntryId::new(5);
        state.apply(AgentEvent::ReasoningDelta {
            turn_id: 1,
            text: "thought".to_string(),
        });
        state.toggle_last_block();
        state.apply(AgentEvent::TranscriptEntryAdded(TranscriptEntry::new(
            id,
            TranscriptEntryKind::Assistant {
                text: "answer".to_string(),
                reasoning: None,
            },
        )));

        let model = state.render_model();
        assert!(model.transcript.is_expanded(id));
        assert!(matches!(
            model.transcript.rows.first(),
            Some(crate::entities::render_model::transcript::TranscriptRow::Assistant {
                reasoning: Some(reasoning),
                ..
            }) if reasoning == "thought"
        ));
    }

    #[test]
    fn skipped_history_lines_produce_a_non_modal_notice() {
        let mut state = state();
        state.apply(AgentEvent::HistoryLoaded {
            entries: Vec::new(),
            skipped_lines: 2,
        });

        let model = state.render_model();
        assert!(model.modal.is_none());
        assert!(
            model
                .status
                .notices
                .iter()
                .any(|notice| { notice.text.contains("2 unreadable history lines") })
        );
    }

    #[test]
    fn toggle_after_live_reasoning_targets_the_newer_tool_block() {
        let mut state = state();
        let id = EntryId::new(9);
        state.apply(AgentEvent::ReasoningDelta {
            turn_id: 1,
            text: "thought".to_string(),
        });
        state.apply(AgentEvent::TranscriptEntryAdded(TranscriptEntry::new(
            id,
            TranscriptEntryKind::ToolCall {
                call_id: "call-1".to_string(),
                tool_name: "read_file".to_string(),
                arguments: serde_json::json!({}),
            },
        )));
        state.toggle_last_block();

        let model = state.render_model();
        assert!(model.transcript.is_expanded(id));
        assert!(matches!(
            model.transcript.rows.last(),
            Some(
                crate::entities::render_model::transcript::TranscriptRow::LiveReasoning {
                    expanded: false,
                    ..
                }
            )
        ));
    }

    #[test]
    fn notices_and_activity_are_projected_to_the_status() {
        let mut state = state();
        state.apply(AgentEvent::Notice(Notice::new(
            NoticeLevel::Warning,
            "careful",
        )));
        state.apply(AgentEvent::ActivityChanged(AgentActivity::Stopping));

        let model = state.render_model();
        assert_eq!(model.status.activity, AgentActivity::Stopping);
        assert_eq!(model.status.notices.len(), 1);
    }
}
