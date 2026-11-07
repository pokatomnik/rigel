/// Pure application action produced by the interactive command parser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ChatAction {
    /// Exit the interactive session.
    Exit,
    /// Continue waiting for the next input.
    Continue,
    /// Submit prompt text and indicate whether an editor should echo it.
    Prompt {
        /// User prompt text.
        text: String,
        /// Whether the editor-originated prompt should be echoed.
        echo: bool,
    },
    /// Clear conversation history and reset usage.
    New,
    /// Compact the current context.
    Compact,
    /// Request model selection.
    ChangeModel,
    /// Start autonomous goal execution.
    Goal(String),
    /// Ask the workflow to discover and select a skill.
    Skills,
    /// Ask the workflow to open the configured editor.
    Editor,
    /// Ask the workflow to print command help.
    Help,
    /// Reject an unknown or malformed slash command.
    Unknown,
}
