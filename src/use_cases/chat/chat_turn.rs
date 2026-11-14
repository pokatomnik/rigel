use crate::entities::{
    agent_event::{
        event::AgentEvent,
        status::{AgentActivity, NoticeLevel},
    },
    conversation::turn::TurnStatus,
    goal::goal_state::GoalReport,
    transcript::entry::{TranscriptEntry, TranscriptEntryKind},
};

use super::workflow::Chat;

impl Chat {
    /// Runs one prompt through the opaque session and applies post-turn policies.
    pub(super) async fn run_prompt(&self, prompt: String) -> anyhow::Result<Option<GoalReport>> {
        self.compact_before_turn().await?;
        self.emit_activity(AgentActivity::Running);
        self.emit(AgentEvent::TranscriptEntryAdded(TranscriptEntry::new(
            self.allocate_entry_id(),
            TranscriptEntryKind::User {
                text: prompt.clone(),
            },
        )));
        let context_usage = *self.context_usage.lock().await;
        let status = self
            .session
            .lock()
            .await
            .run_turn(prompt, context_usage)
            .await?;
        self.update_context_usage(status.context_usage()).await;
        self.emit(AgentEvent::ContextUsageChanged(status.context_usage()));
        self.compact_after_turn(&status).await?;
        self.report_turn_status(&status);
        self.emit_assistant_entry(&status);
        Ok(self.take_goal_report())
    }

    pub(super) fn report_turn_status(&self, status: &TurnStatus) {
        if let Some(error) = status.error() {
            self.notice(
                NoticeLevel::Error,
                format!("\n{}", format_recovery_stopped_notice(error)),
            );
        }
    }

    fn emit_assistant_entry(&self, status: &TurnStatus) {
        let TurnStatus::Complete { output, .. } = status else {
            return;
        };
        if output.trim().is_empty() {
            return;
        }
        self.emit(AgentEvent::TranscriptEntryAdded(TranscriptEntry::new(
            self.allocate_entry_id(),
            TranscriptEntryKind::Assistant {
                text: output.clone(),
                reasoning: None,
            },
        )));
    }
}

/// Formats the user-facing notice after automatic recovery stops.
///
/// The notice directs the user to provide guidance in a new turn or exit explicitly.
pub(crate) fn format_recovery_stopped_notice(error: &str) -> String {
    format!(
        "[automatic recovery stopped: {error}]\nReview the tool errors above, then enter a new message with instructions for the model, or /exit."
    )
}

#[cfg(test)]
mod tests {
    use super::format_recovery_stopped_notice;

    #[test]
    fn recovery_stopped_notice_tells_user_how_to_resume() {
        let message = format_recovery_stopped_notice("MaxTurnsError");

        assert!(message.contains("automatic recovery stopped: MaxTurnsError"));
        assert!(message.contains("enter a new message with instructions for the model"));
        assert!(message.contains("/exit"));
    }
}
