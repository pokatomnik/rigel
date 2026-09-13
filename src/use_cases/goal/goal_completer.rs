use std::sync::Arc;

use crate::{
    contracts::tools::goal::GoalCompleter,
    entities::goal::goal_state::{GoalReport, GoalState, GoalStateError},
};

/// Contract adapter that exposes one shared goal entity to model tools and hooks.
pub(crate) struct GoalStateCompleter {
    state: Arc<GoalState>,
}

impl GoalStateCompleter {
    /// Creates an adapter around the single session-owned goal state.
    pub(crate) fn new(state: Arc<GoalState>) -> Self {
        Self { state }
    }
}

impl GoalCompleter for GoalStateCompleter {
    fn complete(
        &self,
        report: String,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<GoalReport, GoalStateError>> + Send + '_>,
    > {
        Box::pin(async move { self.state.complete(report) })
    }

    fn has_pending_report(&self) -> bool {
        self.state.has_pending_report()
    }
}
