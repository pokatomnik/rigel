use std::{future::Future, pin::Pin};

use futures::future::BoxFuture;

use crate::entities::{
    conversation::{
        context_usage::ContextUsage,
        turn::{CompactionKind, CompactionResult, TurnStatus},
    },
    model::model_session_kind::ModelSessionKind,
    selected_model::selection::SelectedModel,
};
use crate::infrastructure::ui::gateway::EventSink;
use std::sync::Arc;

/// Runs model operations without exposing `rig::Agent` to use cases.
pub(crate) trait ModelSession: Send + Sync {
    /// Executes one prompt against the session-owned conversation and returns its status.
    fn run_turn(
        &self,
        prompt: String,
        context_usage: ContextUsage,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<TurnStatus>> + Send + '_>>;

    /// Compacts the session-owned conversation without exposing provider types.
    fn compact(
        &self,
        kind: CompactionKind,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<CompactionResult>> + Send + '_>>;

    /// Returns the selected model's context limit for compaction decisions.
    fn max_context_tokens(&self) -> Option<u64>;

    /// Returns the model identity chosen for this session, if known.
    fn selected_model(&self) -> Option<SelectedModel> {
        None
    }

    /// Redirects this session's live events to a scoped sink before the first turn.
    ///
    /// Child sessions use it to translate provider progress into parent-visible subagent
    /// activity. Sessions that do not stream progress keep the default no-op behavior.
    fn attach_events(&mut self, _events: Arc<dyn EventSink>) {}
}

/// Creates complete model sessions transactionally; failed creation must not replace the old one.
pub(crate) trait ModelSessionFactory: Send + Sync {
    /// Builds a session for one selected model/session kind.
    ///
    /// The `Send` future allows independent session creation in background tasks.
    fn create(
        &self,
        kind: ModelSessionKind,
        selected_model: Option<SelectedModel>,
    ) -> BoxFuture<'_, anyhow::Result<Box<dyn ModelSession>>>;
}
