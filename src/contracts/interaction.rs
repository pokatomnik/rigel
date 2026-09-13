use crate::entities::{
    application::input_outcome::InputOutcome, conversation::context_usage::ContextUsage,
    tool::tool_entities::ToolPermissionRequest,
    tool_confirm_result::tool_confirm_result::ToolConfirmResult,
};

/// Supplies selectable answers and free-form input to the `ask_user` tool.
pub(crate) trait UserInteraction: Send + Sync {
    /// Displays a question and returns a selected item index.
    fn select(&self, question: &str, items: &[String]) -> Result<Option<usize>, anyhow::Error>;

    /// Reads a non-empty free-form answer with the current context usage.
    fn readline(&self, context_usage: ContextUsage) -> Result<InputOutcome, anyhow::Error>;
}

/// Converts a tool permission request into a user decision.
pub(crate) trait ToolPermissionPrompter: Send + Sync {
    /// Prompts once and returns an explicit decision, including cancellation as denial.
    fn confirm(&self, request: &ToolPermissionRequest) -> ToolConfirmResult;
}
