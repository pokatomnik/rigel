use std::future::Future;
use std::pin::Pin;

use crate::entities::conversation::{
    conversation_message::ConversationMessage, turn::HistoryUpdate,
};

/// Persists domain messages for one conversation or a no-op subagent session.
pub(crate) trait ConversationPersistence: Send + Sync + 'static {
    /// Commits the candidate snapshot before the in-memory owner changes state.
    fn save(
        &self,
        messages: &[ConversationMessage],
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>>;
}

/// Owns a transactional conversation snapshot.
pub(crate) trait ConversationHistory: Send + Sync {
    /// Returns a stable copy suitable for one model call.
    fn snapshot(&self) -> Pin<Box<dyn Future<Output = Vec<ConversationMessage>> + Send + '_>>;

    /// Applies an append or replacement after successful persistence.
    fn update(
        &self,
        update: HistoryUpdate,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>>;

    /// Marks a model-internal compaction that must not duplicate history callbacks.
    fn begin_compaction(&self);

    /// Releases the compaction marker after the model operation completes.
    fn end_compaction(&self);

    /// Reports whether compaction currently owns the history boundary.
    fn is_compaction_in_progress(&self) -> bool;
}
