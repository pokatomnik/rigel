use crate::entities::{
    application::input_outcome::InputOutcome,
    conversation::{context_usage::ContextUsage, conversation_message::ConversationMessage},
    skill::definition::SkillDescriptor,
    tool::tool_entities::ToolPermissionRequest,
    tool_confirm_result::outcome::ToolConfirmResult,
};

/// Supplies interactive input and output to the chat workflow without exposing a terminal type.
pub(crate) trait ChatIO: Send + Sync {
    /// Reads one prompt line and preserves EOF and cancellation as distinct outcomes.
    fn readline(&self, context_usage: ContextUsage) -> anyhow::Result<InputOutcome>;

    /// Opens the configured editor and returns non-empty text.
    fn editor(&self) -> anyhow::Result<String>;

    /// Writes ordinary output without adding a line ending.
    fn print(&self, message: &str);

    /// Writes a regular user-facing line.
    fn eprintln(&self, message: &str);

    /// Writes a low-priority status line.
    fn eprintln_gray(&self, message: &str);

    /// Writes an error status line.
    fn eprintln_red(&self, message: &str);

    /// Writes a warning status line.
    fn eprintln_orange(&self, message: &str);

    /// Selects one skill descriptor or reports cancellation.
    fn select_skill(&self, items: &[SkillDescriptor]) -> anyhow::Result<Option<usize>>;

    /// Renders the loaded history before the first interactive turn.
    fn display_history(&self, messages: &[ConversationMessage]);
}

/// Supplies selectable answers and free-form input to interactive tools.
pub(crate) trait UserInteraction: Send + Sync {
    /// Displays a question and returns a selected item index.
    fn select(&self, question: &str, items: &[String]) -> Result<Option<usize>, anyhow::Error>;

    /// Reads a non-empty free-form answer with the current context usage.
    fn readline(&self, context_usage: ContextUsage) -> Result<InputOutcome, anyhow::Error>;
}

/// Prompts for an explicit tool permission decision at an interactive boundary.
pub(crate) trait ToolPermissionPrompter: Send + Sync {
    /// Returns a domain confirmation result without exposing provider actions.
    fn confirm(&self, request: &ToolPermissionRequest) -> ToolConfirmResult;
}
