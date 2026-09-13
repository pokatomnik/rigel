use std::{
    error::Error,
    fmt::{self, Display},
};

/// Failure categories reported by an interactive user-selection capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InteractionFailure {
    /// The user explicitly cancelled the interaction.
    Cancelled,
    /// The host cannot provide an interactive terminal.
    Unavailable,
}

impl Display for InteractionFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Cancelled => "the user cancelled the interaction",
            Self::Unavailable => "the user interaction is unavailable",
        };
        formatter.write_str(message)
    }
}

impl Error for InteractionFailure {}
