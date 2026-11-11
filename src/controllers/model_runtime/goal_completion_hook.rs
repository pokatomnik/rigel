use std::sync::Arc;

use rig::agent::{AgentHook, CompletionCallAction, CompletionCallEvent, HookContext, RequestPatch};
use rig::message::ToolChoice;

use crate::infrastructure::tools::goal::GoalCompleter;

/// Thin Rig adapter that forces a final answer after a goal completion report.
pub(crate) struct GoalCompletionHook {
    goal_completer: Arc<dyn GoalCompleter>,
}

impl GoalCompletionHook {
    /// Creates a hook around the session-owned goal capability.
    pub(crate) fn new(goal_completer: Arc<dyn GoalCompleter>) -> Self {
        Self { goal_completer }
    }
}

impl AgentHook for GoalCompletionHook {
    async fn on_completion_call(
        &self,
        _ctx: &HookContext,
        _event: CompletionCallEvent<'_>,
    ) -> CompletionCallAction {
        if self.goal_completer.has_pending_report() {
            return CompletionCallAction::patch(RequestPatch::new().tool_choice(ToolChoice::None));
        }
        CompletionCallAction::continue_run()
    }
}
