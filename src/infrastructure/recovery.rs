use crate::entities::{
    conversation::{context_usage::ContextUsage, turn::TurnStatus},
    recovery::{
        InvalidToolCallDecision, ModelTurnDecision, RecoveryOutcome, RecoveryRequest,
        ToolResultDecision, ToolResultObservation,
    },
};
use futures::future::BoxFuture;

/// Executes one recovery prompt through a provider-specific model path.
pub(crate) trait RecoveryTurn: Send + Sync {
    /// Runs a prompt against the current session history and returns provider-neutral outcome data.
    fn run_prompt(&self, prompt: String) -> BoxFuture<'_, anyhow::Result<RecoveryOutcome>>;
}

/// Applies provider-neutral recovery policy and retry budgets.
pub(crate) trait TurnRecovery: Send + Sync {
    /// Recovers an incomplete turn using the supplied runner and initial token usage.
    fn recover<'a>(
        &'a self,
        request: RecoveryRequest,
        runner: &'a dyn RecoveryTurn,
        context_usage: ContextUsage,
    ) -> BoxFuture<'a, anyhow::Result<TurnStatus>>;
}

/// Stateful recovery decisions scoped to one model-agent run.
pub(crate) trait ToolRecoveryPolicy: Send + Sync {
    /// Clones the policy value so Rig scratchpad snapshots preserve run-local state.
    fn clone_box(&self) -> Box<dyn ToolRecoveryPolicy>;
    /// Records a tool outcome and decides whether to expose a recovery marker.
    fn observe(&mut self, observation: ToolResultObservation) -> ToolResultDecision;

    /// Decides whether an empty model turn needs recovery.
    fn decide_model_turn(&mut self, has_tool_call: bool, has_answer: bool) -> ModelTurnDecision;

    /// Counts an invalid call and applies the fixed retry budget.
    fn invalid_call(&mut self) -> InvalidToolCallDecision;

    /// Returns whether the next completion must forbid tools.
    fn force_final(&self) -> Option<String>;
}

/// Creates independent recovery policy state for each Rig run.
pub(crate) trait ToolRecoveryPolicyFactory: Send + Sync {
    /// Creates a fresh mutable policy with empty budgets and invocation history.
    fn create(&self) -> Box<dyn ToolRecoveryPolicy>;
}
