pub(crate) const MAX_INVALID_TOOL_CALL_ATTEMPTS: usize = 2;

/// Cause selected by the runtime when a completed turn requires recovery.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RecoveryRequest {
    /// The provider stream failed and supplies its latest diagnostic.
    StreamFailure(String),
    /// The model completed reasoning without a non-empty final answer.
    MissingAnswer,
    /// A tool failure was not followed by a corrective call or final answer.
    UnresolvedTool,
}

/// Provider-neutral result of one recovery prompt execution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RecoveryOutcome {
    /// A model completion, including an empty one during stream-failure recovery.
    Completed {
        /// Final model text, which may be empty for stream-failure recovery.
        output: String,
        /// Whether final-answer workflows may accept this output.
        has_output: bool,
        /// Most recent usage reported by this completion.
        context_usage: crate::entities::conversation::context_usage::ContextUsage,
    },
    /// A failed recovery prompt execution.
    Failed {
        /// Diagnostic retained for later retries or final status.
        error: String,
        /// Most recent usage reported by this failure.
        context_usage: crate::entities::conversation::context_usage::ContextUsage,
    },
}

impl RecoveryOutcome {
    /// Reports whether this completion is acceptable to final-answer recovery flows.
    pub(crate) fn has_output(&self) -> bool {
        matches!(
            self,
            Self::Completed {
                has_output: true,
                ..
            }
        )
    }

    /// Converts a completion to turn status, or returns a failure's diagnostic and usage.
    pub(crate) fn into_completed_status(
        self,
    ) -> Result<
        crate::entities::conversation::turn::TurnStatus,
        (
            String,
            crate::entities::conversation::context_usage::ContextUsage,
        ),
    > {
        match self {
            Self::Completed {
                output,
                context_usage,
                ..
            } => Ok(crate::entities::conversation::turn::TurnStatus::complete(
                output,
                context_usage,
            )),
            Self::Failed {
                error,
                context_usage,
            } => Err((error, context_usage)),
        }
    }
}

/// One provider-neutral observation of a tool invocation and its result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ToolResultObservation {
    /// Name used to identify retries of the same tool.
    pub(crate) tool_name: String,
    /// Serialized arguments used for cycle and duplicate detection.
    pub(crate) args: String,
    /// Zero-based runtime turn associated with this result.
    pub(crate) turn: usize,
    /// Whether the tool result represents an execution failure.
    pub(crate) failed: bool,
    /// Whether the tool result represents successful execution.
    pub(crate) successful: bool,
    /// Whether successful execution was a semantic no-op.
    pub(crate) no_op: bool,
    /// Rendered diagnostic retained for recovery guidance.
    pub(crate) diagnostic: String,
}

impl ToolResultObservation {
    /// Starts an observation with a tool identity and turn; outcome flags default to false.
    pub(crate) fn new(tool_name: &str, args: &str, turn: usize) -> Self {
        Self {
            tool_name: tool_name.to_string(),
            args: args.to_string(),
            turn,
            failed: false,
            successful: false,
            no_op: false,
            diagnostic: String::new(),
        }
    }

    /// Marks the observation failed and stores its rendered diagnostic.
    pub(crate) fn failed(mut self, diagnostic: &str) -> Self {
        self.failed = true;
        self.diagnostic = diagnostic.to_string();
        self
    }

    /// Marks the observation successful and stores its rendered diagnostic.
    pub(crate) fn successful(mut self, diagnostic: &str) -> Self {
        self.successful = true;
        self.diagnostic = diagnostic.to_string();
        self
    }

    /// Marks an otherwise successful result as a no-op.
    pub(crate) fn no_op(mut self) -> Self {
        self.no_op = true;
        self
    }
}

/// Pending failed tool operation retained until a later turn resolves it.
#[derive(Clone, Debug)]
pub(crate) struct PendingToolFailure {
    /// Tool identity required for recovery matching.
    pub(crate) tool_name: String,
    /// Diagnostic shown in recovery guidance.
    pub(crate) diagnostic: String,
    /// Turn in which the failed tool result was observed.
    pub(crate) failed_turn: usize,
}

/// Tool identity and argument snapshot used by the cycle guard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ToolInvocationFingerprint {
    /// Invoked tool name.
    pub(crate) tool_name: String,
    /// Serialized tool arguments.
    pub(crate) args: String,
}

/// Failed invocation identity used by the repeated-failure guard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ToolFailureFingerprint {
    /// Failed tool name.
    pub(crate) tool_name: String,
    /// Serialized failed arguments.
    pub(crate) args: String,
    /// Rendered failure diagnostic.
    pub(crate) diagnostic: String,
}

/// Policy response to an observed tool result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ToolResultDecision {
    /// Leaves the model-visible tool result unchanged.
    Keep,
    /// Marks a failed invocation and includes budget/forced-final state.
    ReportFailure {
        /// Number of failed executions observed in this run.
        total_failures: usize,
        /// Present when the next completion must not call tools.
        force_final_answer: Option<String>,
    },
    /// Marks a same-tool later-turn success as recovery.
    ReportRecovery,
}

/// Policy response after a model turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ModelTurnDecision {
    /// Allows the model run to continue without recovery feedback.
    Continue,
    /// Retries the model turn with targeted recovery instructions.
    Retry {
        /// Model-visible retry prompt.
        prompt: String,
    },
    /// Stops the run after the recovery budget is exhausted.
    Stop {
        /// User/model-visible stopping diagnostic.
        message: String,
    },
}

/// Response to an invalid tool call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum InvalidToolCallDecision {
    /// Requests another model response after an invalid tool dispatch.
    Retry {
        /// One-based retry attempt number.
        attempt: usize,
        /// Maximum number of invalid-call retries allowed.
        max_attempts: usize,
        /// Explains why this response must be a final answer when forced.
        force_final_answer: Option<String>,
    },
    /// Stops after the invalid-call retry budget has been exceeded.
    Stop {
        /// One-based invalid-call attempt number.
        attempt: usize,
        /// Maximum number of invalid-call retries allowed.
        max_attempts: usize,
    },
}
