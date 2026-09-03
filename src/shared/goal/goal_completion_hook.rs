use std::sync::Arc;

use rig::agent::{AgentHook, CompletionCallAction, CompletionCallEvent, HookContext, RequestPatch};
use rig::message::ToolChoice;

use super::goal_state::GoalState;

/// Prevents another tool call after the completion event in the same run.
pub(crate) struct GoalCompletionHook {
    goal_state: Arc<GoalState>,
}

impl GoalCompletionHook {
    pub(crate) fn new(goal_state: Arc<GoalState>) -> Self {
        Self { goal_state }
    }
}

impl AgentHook for GoalCompletionHook {
    async fn on_completion_call(
        &self,
        _ctx: &HookContext,
        _event: CompletionCallEvent<'_>,
    ) -> CompletionCallAction {
        if self.goal_state.has_pending_report() {
            return CompletionCallAction::patch(RequestPatch::new().tool_choice(ToolChoice::None));
        }
        CompletionCallAction::continue_run()
    }
}
