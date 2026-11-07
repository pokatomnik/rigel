use std::future::Future;
use std::pin::Pin;

use crate::entities::tool::tool_entities::{SubagentReport, SubagentTask};

/// Runs a fresh, non-persistent subagent session without exposing its runtime construction.
pub(crate) trait SubagentRunner: Send + Sync {
    /// Executes the task with the reduced subagent tool set.
    fn run(
        &self,
        task: SubagentTask,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<SubagentReport>> + Send + '_>>;
}
