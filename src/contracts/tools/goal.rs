use std::future::Future;
use std::pin::Pin;

use crate::entities::{goal::goal_state::GoalReport, goal::goal_state::GoalStateError};

/// Completes the single active goal owned by the application session.
pub(crate) trait GoalCompleter: Send + Sync {
    /// Stores a truthful report, rejecting completion when no goal is active.
    fn complete(
        &self,
        report: String,
    ) -> Pin<Box<dyn Future<Output = Result<GoalReport, GoalStateError>> + Send + '_>>;

    /// Returns whether completion has produced a pending report boundary.
    fn has_pending_report(&self) -> bool;
}
