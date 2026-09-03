use rig::completion::CompletionModel;

use crate::{
    entities::context_usage::ContextUsage,
    prompts::recovery::{
        missing_answer_recovery_prompt, recovery_prompt, unresolved_tool_recovery_prompt,
    },
    shared::{
        history::history_persistence::HistoryPersistence,
        streaming::streamed_turn::{StreamRunOutcome, StreamedTurn},
    },
};

const MAX_RECOVERY_ATTEMPTS: usize = 3;
const MAX_TOOL_RECOVERY_ATTEMPTS: usize = 2;

pub(crate) enum RecoveryRequest {
    StreamFailure(String),
    MissingAnswer,
    UnresolvedTool,
}

#[allow(dead_code)]
pub(crate) enum TurnStatus {
    Complete {
        output: String,
        context_usage: ContextUsage,
    },
    RecoveryStopped {
        error: String,
        context_usage: ContextUsage,
    },
}

impl TurnStatus {
    pub(crate) fn context_usage(&self) -> ContextUsage {
        match self {
            Self::Complete { context_usage, .. } | Self::RecoveryStopped { context_usage, .. } => {
                *context_usage
            }
        }
    }

    pub(crate) fn output(&self) -> Option<&str> {
        match self {
            Self::Complete { output, .. } => Some(output.as_str()),
            Self::RecoveryStopped { .. } => None,
        }
    }

    pub(crate) fn error(&self) -> Option<&str> {
        match self {
            Self::Complete { .. } => None,
            Self::RecoveryStopped { error, .. } => Some(error.as_str()),
        }
    }
}

/// Restores an incomplete model turn through the normal durable stream path.
pub(crate) struct TurnRecoverer<'a, CM, P>
where
    CM: CompletionModel,
    P: HistoryPersistence,
{
    streamed_turn: StreamedTurn<'a, CM, P>,
}

impl<'a, CM, P> TurnRecoverer<'a, CM, P>
where
    CM: CompletionModel + 'static,
    P: HistoryPersistence,
{
    pub(crate) fn new(streamed_turn: StreamedTurn<'a, CM, P>) -> Self {
        Self { streamed_turn }
    }

    pub(crate) async fn recover(
        &self,
        request: RecoveryRequest,
        context_usage: ContextUsage,
    ) -> anyhow::Result<TurnStatus> {
        match request {
            RecoveryRequest::StreamFailure(error) => {
                self.recover_stream_failure(error, context_usage).await
            }
            RecoveryRequest::MissingAnswer => self.recover_missing_answer(context_usage).await,
            RecoveryRequest::UnresolvedTool => self.recover_unresolved_tool(context_usage).await,
        }
    }

    async fn recover_stream_failure(
        &self,
        initial_error: String,
        mut context_usage: ContextUsage,
    ) -> anyhow::Result<TurnStatus> {
        let mut error = initial_error;
        for attempt in 1..=MAX_RECOVERY_ATTEMPTS {
            let prompt = recovery_prompt(error.as_str(), attempt, MAX_RECOVERY_ATTEMPTS);
            match self.run_prompt(prompt).await? {
                StreamRunOutcome::Completed(completion) => {
                    context_usage = completion.context_usage();
                    return Ok(TurnStatus::Complete {
                        output: completion.output().to_string(),
                        context_usage,
                    });
                }
                StreamRunOutcome::Failed(failure) => {
                    context_usage = failure.context_usage();
                    error = failure.error().to_string();
                }
            }
        }

        Ok(TurnStatus::RecoveryStopped {
            error: format!(
                "model failed to recover after {MAX_RECOVERY_ATTEMPTS} attempts: {error}"
            ),
            context_usage,
        })
    }

    async fn recover_missing_answer(
        &self,
        mut context_usage: ContextUsage,
    ) -> anyhow::Result<TurnStatus> {
        let mut last_error = None;
        for attempt in 1..=MAX_RECOVERY_ATTEMPTS {
            let prompt = missing_answer_recovery_prompt(attempt, MAX_RECOVERY_ATTEMPTS);
            match self.run_prompt(prompt).await? {
                StreamRunOutcome::Completed(completion) if completion.has_output() => {
                    context_usage = completion.context_usage();
                    return Ok(TurnStatus::Complete {
                        output: completion.output().to_string(),
                        context_usage,
                    });
                }
                StreamRunOutcome::Completed(completion) => {
                    context_usage = completion.context_usage();
                    last_error = None;
                }
                StreamRunOutcome::Failed(failure) => {
                    context_usage = failure.context_usage();
                    last_error = Some(failure.error().to_string());
                }
            }
        }

        Ok(TurnStatus::RecoveryStopped {
            error: missing_answer_error(last_error),
            context_usage,
        })
    }

    async fn recover_unresolved_tool(
        &self,
        mut context_usage: ContextUsage,
    ) -> anyhow::Result<TurnStatus> {
        let mut last_error = None;
        for attempt in 1..=MAX_TOOL_RECOVERY_ATTEMPTS {
            let prompt = unresolved_tool_recovery_prompt(attempt, MAX_TOOL_RECOVERY_ATTEMPTS);
            match self.run_prompt(prompt).await? {
                StreamRunOutcome::Completed(completion) if completion.has_output() => {
                    context_usage = completion.context_usage();
                    return Ok(TurnStatus::Complete {
                        output: completion.output().to_string(),
                        context_usage,
                    });
                }
                StreamRunOutcome::Completed(completion) => {
                    context_usage = completion.context_usage();
                    last_error = None;
                }
                StreamRunOutcome::Failed(failure) => {
                    context_usage = failure.context_usage();
                    last_error = Some(failure.error().to_string());
                }
            }
        }

        Ok(TurnStatus::RecoveryStopped {
            error: unresolved_tool_error(last_error),
            context_usage,
        })
    }

    async fn run_prompt(&self, prompt: String) -> anyhow::Result<StreamRunOutcome> {
        self.streamed_turn.run_latest(prompt).await
    }
}

fn missing_answer_error(last_error: Option<String>) -> String {
    match last_error {
        Some(error) => format!(
            "model failed to produce an answer after {MAX_RECOVERY_ATTEMPTS} recovery attempts: \
             {error}"
        ),
        None => format!("model produced no answer after {MAX_RECOVERY_ATTEMPTS} recovery attempts"),
    }
}

fn unresolved_tool_error(last_error: Option<String>) -> String {
    match last_error {
        Some(error) => format!(
            "model failed to correct a tool error after {MAX_TOOL_RECOVERY_ATTEMPTS} recovery \
             attempts: {error}"
        ),
        None => format!(
            "model produced no answer after correcting a tool error in \
             {MAX_TOOL_RECOVERY_ATTEMPTS} recovery attempts"
        ),
    }
}
