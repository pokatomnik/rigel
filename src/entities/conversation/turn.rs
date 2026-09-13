use super::context_usage::ContextUsage;

/// Atomic history changes used by persistence-aware conversation state.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum HistoryUpdate {
    /// Append one message to the current history.
    Append(super::conversation_message::ConversationMessage),
    /// Replace the complete history snapshot.
    Replace(Vec<super::conversation_message::ConversationMessage>),
}

/// Observable result of one turn after normal execution or recovery.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum TurnStatus {
    /// A non-empty answer was produced.
    Complete {
        /// Final model text.
        output: String,
        /// Latest provider-neutral usage information.
        context_usage: ContextUsage,
    },
    /// Recovery budgets stopped the turn without an answer.
    RecoveryStopped {
        /// Actionable model/user-facing error.
        error: String,
        /// Latest provider-neutral usage information.
        context_usage: ContextUsage,
    },
}

impl TurnStatus {
    /// Returns usage for either successful or stopped turns.
    pub(crate) fn context_usage(&self) -> ContextUsage {
        match self {
            Self::Complete { context_usage, .. } | Self::RecoveryStopped { context_usage, .. } => {
                *context_usage
            }
        }
    }

    /// Returns answer text only for successful turns.
    pub(crate) fn output(&self) -> Option<&str> {
        match self {
            Self::Complete { output, .. } => Some(output.as_str()),
            Self::RecoveryStopped { .. } => None,
        }
    }

    /// Returns the stopping error only for failed recovery.
    pub(crate) fn error(&self) -> Option<&str> {
        match self {
            Self::Complete { .. } => None,
            Self::RecoveryStopped { error, .. } => Some(error.as_str()),
        }
    }
}

/// Identifies whether compaction was requested by a user or by a threshold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CompactionKind {
    /// Explicit `/compact` request.
    Manual,
    /// Automatic compaction at the configured threshold.
    Automatic,
}

/// Reports whether a compaction replaced conversation history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CompactionResult {
    kind: CompactionKind,
    compacted: bool,
}

impl CompactionResult {
    /// Creates a result for one compaction attempt.
    pub(crate) const fn new(kind: CompactionKind, compacted: bool) -> Self {
        Self { kind, compacted }
    }

    /// Returns the trigger for the attempt.
    pub(crate) const fn kind(self) -> CompactionKind {
        self.kind
    }

    /// Returns whether new summarized history was committed.
    pub(crate) const fn compacted(self) -> bool {
        self.compacted
    }
}
