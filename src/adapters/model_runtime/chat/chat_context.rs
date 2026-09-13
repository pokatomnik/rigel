use crate::{
    adapters::model_runtime::chat::history::{
        chat_history::HistoryUpdate, history_persistence::HistoryPersistence,
    },
    entities::conversation::{
        context_usage::ContextUsage,
        turn::{CompactionKind, CompactionResult, TurnStatus},
    },
};

use super::chat::Chat;

impl<P> Chat<P>
where
    P: HistoryPersistence,
{
    pub(super) async fn history_updated(&self, update: HistoryUpdate) -> anyhow::Result<()> {
        self.history.update(update).await
    }

    pub(super) async fn compact_context(&self) -> anyhow::Result<bool> {
        let result = self.compact_context_kind(CompactionKind::Manual).await?;
        if result.kind() == CompactionKind::Manual && result.compacted() {
            self.reset_used_tokens().await;
        }
        Ok(result.compacted())
    }

    pub(super) async fn handle_compaction(&self) -> anyhow::Result<()> {
        self.terminal_io.eprintln_gray("Compacting started...");
        if self.compact_context().await? {
            self.terminal_io.eprintln_gray("Compacting done.");
        } else {
            self.terminal_io.eprintln_gray("Compacting failed.");
        }
        Ok(())
    }

    async fn compact_context_kind(&self, kind: CompactionKind) -> anyhow::Result<CompactionResult> {
        self.session.lock().await.compact(kind).await
    }

    pub(super) async fn compact_after_turn(&self, status: &TurnStatus) -> anyhow::Result<()> {
        if !matches!(status, TurnStatus::Complete { .. })
            || !status.context_usage().should_compact()
        {
            return Ok(());
        }

        self.terminal_io.eprintln_red("Compacting context...");
        let result = self
            .session
            .lock()
            .await
            .compact(CompactionKind::Automatic)
            .await?;
        if result.kind() == CompactionKind::Automatic && result.compacted() {
            self.reset_used_tokens().await;
            self.terminal_io.eprintln_red("Context compacted.");
        } else {
            self.terminal_io.eprintln_red("Compacting failed.");
        }
        Ok(())
    }

    pub(super) async fn update_context_usage(&self, context_usage: ContextUsage) {
        *self.context_usage.lock().await = context_usage;
    }

    pub(super) async fn reset_used_tokens(&self) {
        let mut context_usage = self.context_usage.lock().await;
        *context_usage = ContextUsage::new(None, context_usage.max_context_tokens());
    }
}
