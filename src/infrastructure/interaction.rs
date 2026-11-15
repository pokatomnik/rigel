use futures::future::BoxFuture;

use crate::entities::{
    application::input_outcome::InputOutcome, conversation::context_usage::ContextUsage,
    tool::tool_entities::ToolPermissionRequest, tool_confirm_result::outcome::ToolConfirmResult,
};

/// Supplies interactive input and output to the chat workflow without exposing a terminal type.
pub(crate) trait ChatIO: Send + Sync {
    /// Writes an error status line.
    fn eprintln_red(&self, message: &str);
}

/// Supplies selectable answers and free-form input to interactive tools.
pub(crate) trait UserInteraction: Send + Sync {
    /// Displays a question and returns a selected item index.
    fn select<'a>(
        &'a self,
        question: &'a str,
        items: &'a [String],
    ) -> BoxFuture<'a, anyhow::Result<Option<usize>>>;

    /// Reads a non-empty free-form answer with the current context usage.
    fn readline<'a>(
        &'a self,
        context_usage: ContextUsage,
    ) -> BoxFuture<'a, anyhow::Result<InputOutcome>>;
}

/// Prompts for an explicit tool permission decision at an interactive boundary.
pub(crate) trait ToolPermissionPrompter: Send + Sync {
    /// Returns a domain confirmation result without exposing provider actions.
    fn confirm<'a>(
        &'a self,
        request: &'a ToolPermissionRequest,
    ) -> BoxFuture<'a, ToolConfirmResult>;
}
