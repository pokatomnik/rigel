use futures::future::BoxFuture;

use crate::entities::tool_confirm_result::outcome::ToolConfirmResult;

/// Resolves interaction requests that the agent is currently awaiting.
pub(crate) trait InteractionBroker: Send + Sync {
    /// Delivers one permission decision; returns false when the request is no longer pending.
    fn resolve_permission(
        &self,
        correlation_id: u64,
        decision: ToolConfirmResult,
    ) -> BoxFuture<'_, bool>;

    /// Delivers one free-form answer; returns false when the request is no longer pending.
    fn resolve_question(&self, correlation_id: u64, answer: String) -> BoxFuture<'_, bool>;

    /// Cancels one outstanding question; returns false when the request is no longer pending.
    fn cancel_question(&self, correlation_id: u64) -> BoxFuture<'_, bool>;

    /// Cancels every outstanding interaction; returns the number of requests withdrawn.
    fn cancel_all(&self) -> BoxFuture<'_, usize>;
}
