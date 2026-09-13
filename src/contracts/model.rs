use std::future::Future;
use std::pin::Pin;

/// Future returned while a provider-neutral model session is being built.
pub(crate) type ModelSessionBuildFuture<'a> =
    Pin<Box<dyn Future<Output = anyhow::Result<Box<dyn ModelSession>>> + 'a>>;

use crate::entities::{
    conversation::{
        context_usage::ContextUsage,
        turn::{CompactionKind, CompactionResult, TurnStatus},
    },
    selected_model::selected_model::SelectedModel,
};

/// Lists provider models without exposing transport response DTOs.
pub(crate) trait ModelCatalog: Send + Sync {
    /// Fetches available models; an empty result is a user-visible selection error.
    fn models(
        &self,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<SelectedModel>>> + Send + '_>>;
}

/// Selects one model from a domain list using a presentation adapter.
pub(crate) trait ModelSelector: Send + Sync {
    /// Chooses a model while preserving provider context length metadata.
    fn select(
        &self,
        models: &[SelectedModel],
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<SelectedModel>> + Send + '_>>;
}

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
}

/// Identifies why a new isolated model session is being requested.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ModelSessionKind {
    /// Initial interactive chat session.
    Chat,
    /// Replacement after `/agent`.
    ModelChange,
    /// Fresh non-persistent delegated task.
    Subagent,
}

/// Creates complete model sessions transactionally; failed creation must not replace the old one.
pub(crate) trait ModelSessionFactory: Send + Sync {
    /// Builds a session for one selected model/session kind.
    fn create(
        &self,
        kind: ModelSessionKind,
        selected_model: Option<SelectedModel>,
    ) -> ModelSessionBuildFuture<'_>;
}
