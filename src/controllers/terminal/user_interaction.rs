use std::{
    io::{self, IsTerminal},
    sync::Arc,
};

use crate::{
    controllers::terminal::terminal_io::{ReadlineOutcome, TerminalIO},
    entities::{
        application::input_outcome::InputOutcome, conversation::context_usage::ContextUsage,
        tool::interaction_failure::InteractionFailure,
    },
    infrastructure::interaction::UserInteraction,
};

/// Adapts the process terminal to the provider-neutral user interaction contract.
pub(crate) struct TerminalInteraction {
    terminal_io: Arc<TerminalIO>,
}

impl TerminalInteraction {
    /// Creates an interaction adapter for one terminal session.
    pub(crate) fn new(terminal_io: Arc<TerminalIO>) -> Self {
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
    fn select(&self, question: &str, items: &[String]) -> Result<Option<usize>, anyhow::Error> {
        Self::ensure_interactive().map_err(anyhow::Error::new)?;
        dialoguer::Select::new()
            .with_prompt(question)
            .items(items)
            .default(0)
            .interact_opt()
            .map_err(|error| {
                let io_error: io::Error = error.into();
                let failure = if io_error.kind() == io::ErrorKind::Interrupted {
                    InteractionFailure::Cancelled
                } else {
                    InteractionFailure::Unavailable
                };
                anyhow::Error::new(failure)
            })
    }

    fn readline(&self, context_usage: ContextUsage) -> Result<InputOutcome, anyhow::Error> {
        self.terminal_io
            .readline(context_usage)
            .map(map_readline_outcome)
            .map_err(|_| anyhow::Error::new(InteractionFailure::Unavailable))
    }
}

fn map_readline_outcome(outcome: ReadlineOutcome) -> InputOutcome {
    match outcome {
        ReadlineOutcome::Text(answer) => InputOutcome::Text(answer),
        ReadlineOutcome::Eof => InputOutcome::Eof,
        ReadlineOutcome::Cancelled => InputOutcome::Cancelled,
    }
}
