use crate::entities::{
    agent_event::{
        event::AgentEvent,
        status::{AgentActivity, NoticeLevel},
    },
    conversation::{
        context_usage::ContextUsage,
        turn::{CompactionKind, CompactionResult, TurnStatus},
    },
};

use super::workflow::Chat;

impl Chat {
    /// Clears every history segment and starts the conversation from scratch.
    pub(super) async fn handle_new(&self) -> anyhow::Result<()> {
        self.history.reset().await?;
        self.reset_used_tokens().await;
        self.emit(AgentEvent::HistoryLoaded {
            entries: Vec::new(),
            skipped_lines: 0,
        });
        Ok(())
    }

    pub(super) async fn compact_context(&self) -> anyhow::Result<bool> {
        let result = self.compact_context_kind(CompactionKind::Manual).await?;
        if result.kind() == CompactionKind::Manual && result.compacted() {
            self.reset_used_tokens().await;
        }
        Ok(result.compacted())
    }

    async fn compact_context_kind(&self, kind: CompactionKind) -> anyhow::Result<CompactionResult> {
        self.session.lock().await.compact(kind).await
    }

    pub(super) async fn compact_after_turn(&self, status: &TurnStatus) -> anyhow::Result<()> {
        if !status.context_usage().should_compact() {
            return Ok(());
        }

        self.run_automatic_compaction().await
    }

    pub(super) async fn compact_before_turn(&self) -> anyhow::Result<()> {
        let usage = *self.context_usage.lock().await;
        if !usage.should_compact() {
            return Ok(());
        }
        self.run_automatic_compaction().await
    }

    async fn run_automatic_compaction(&self) -> anyhow::Result<()> {
        self.emit_activity(AgentActivity::Compacting);
        self.emit(AgentEvent::CompactionStarted {
            kind: CompactionKind::Automatic,
        });
        let result = self
            .session
            .lock()
            .await
            .compact(CompactionKind::Automatic)
            .await;
        self.emit_activity(AgentActivity::Idle);
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                self.emit(AgentEvent::CompactionFailed {
                    kind: CompactionKind::Automatic,
                    error: error.to_string(),
                });
                self.notice(NoticeLevel::Error, error.to_string());
                return Err(error);
            }
        };
        if result.kind() == CompactionKind::Automatic && result.compacted() {
            self.reset_used_tokens().await;
        }
        self.emit(AgentEvent::CompactionFinished {
            kind: CompactionKind::Automatic,
            compacted: result.compacted(),
        });
        Ok(())
    }

    pub(super) async fn update_context_usage(&self, context_usage: ContextUsage) {
        *self.context_usage.lock().await = context_usage;
    }

    pub(super) async fn reset_used_tokens(&self) {
        let mut context_usage = self.context_usage.lock().await;
        *context_usage = ContextUsage::new(None, context_usage.max_context_tokens());
        self.emit(AgentEvent::ContextUsageChanged(*context_usage));
    }
}
