use std::{
    io::{self, IsTerminal},
    sync::Arc,
};

use crate::{
    entities::context_usage::ContextUsage,
    shared::terminal::terminal_io::{ReadlineOutcome, TerminalIO},
};

/// Failure categories reported by the terminal interaction adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum InteractionFailure {
    /// The user explicitly cancelled the interaction.
    Cancelled,
    /// The host cannot provide an interactive terminal.
    Unavailable,
}

/// Testable boundary for the selection and free-form input UI.
pub(super) trait UserInteraction: Send + Sync {
    /// Displays the question and returns the selected menu index, if any.
    fn select(&self, question: &str, items: &[String])
    -> Result<Option<usize>, InteractionFailure>;

    /// Reads one free-form line using the supplied context usage.
    fn readline(&self, context_usage: ContextUsage) -> Result<ReadlineOutcome, InteractionFailure>;
}

/// Production adapter backed by `dialoguer` and `TerminalIO`.
pub(super) struct TerminalInteraction {
    terminal_io: Arc<TerminalIO>,
}

impl TerminalInteraction {
    /// Creates an adapter for the process terminal used by the chat session.
    pub(super) fn new(terminal_io: Arc<TerminalIO>) -> Self {
        Self { terminal_io }
    }

    fn ensure_interactive() -> Result<(), InteractionFailure> {
        if console::Term::stderr().is_term() && io::stdin().is_terminal() {
            return Ok(());
        }
        Err(InteractionFailure::Unavailable)
    }
}

impl UserInteraction for TerminalInteraction {
    fn select(
        &self,
        question: &str,
        items: &[String],
    ) -> Result<Option<usize>, InteractionFailure> {
        Self::ensure_interactive()?;
        dialoguer::Select::new()
            .with_prompt(question)
            .items(items)
            .default(0)
            .interact_opt()
            .map_err(|error| {
                let io_error: io::Error = error.into();
                if io_error.kind() == io::ErrorKind::Interrupted {
                    InteractionFailure::Cancelled
                } else {
                    InteractionFailure::Unavailable
                }
            })
    }

    fn readline(&self, context_usage: ContextUsage) -> Result<ReadlineOutcome, InteractionFailure> {
        match self.terminal_io.readline(context_usage) {
            Ok(ReadlineOutcome::Cancelled) => Err(InteractionFailure::Cancelled),
            Ok(ReadlineOutcome::Eof) => Err(InteractionFailure::Unavailable),
            Ok(outcome) => Ok(outcome),
            Err(_) => Err(InteractionFailure::Unavailable),
        }
    }
}
