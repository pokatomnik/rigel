use std::future::Future;
use std::pin::Pin;

use crate::entities::tool::tool_entities::{CommandExecutionRequest, CommandExecutionResult};

/// Executes bounded commands through a composition-root-selected process adapter.
pub(crate) trait CommandExecutor: Send + Sync {
    /// Runs a command with adapter-owned shell, timeout, and output limits.
    fn execute(
        &self,
        request: CommandExecutionRequest,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<CommandExecutionResult>> + Send + '_>>;
}

/// Records command/file operation telemetry without global mutable state.
pub(crate) trait CommandUsageRecorder: Send + Sync {
    /// Records one candidate operation for the current session.
    fn record(&self, operation: &str);
}
