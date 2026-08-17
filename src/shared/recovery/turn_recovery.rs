use rig::completion::CompletionModel;

use crate::{
    prompts::recovery::{
        missing_answer_recovery_prompt, recovery_prompt, unresolved_tool_recovery_prompt,
    },
    shared::{
        history::HistoryPersistence,
        streaming::{StreamRunOutcome, StreamedTurn},
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
    Complete(String),
    RecoveryStopped(String),
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

    pub(crate) async fn recover(&self, request: RecoveryRequest) -> anyhow::Result<TurnStatus> {
        match request {
            RecoveryRequest::StreamFailure(error) => self.recover_stream_failure(error).await,
            RecoveryRequest::MissingAnswer => self.recover_missing_answer().await,
            RecoveryRequest::UnresolvedTool => self.recover_unresolved_tool().await,
        }
    }

    async fn recover_stream_failure(&self, initial_error: String) -> anyhow::Result<TurnStatus> {
        let mut error = initial_error;
        for attempt in 1..=MAX_RECOVERY_ATTEMPTS {
            let prompt = recovery_prompt(error.as_str(), attempt, MAX_RECOVERY_ATTEMPTS);
            match self.run_prompt(prompt).await? {
                StreamRunOutcome::Completed(completion) => {
                    return Ok(TurnStatus::Complete(completion.output().to_string()));
                }
                StreamRunOutcome::Failed(next_error) => error = next_error,
            }
        }

        Ok(TurnStatus::RecoveryStopped(format!(
            "model failed to recover after {MAX_RECOVERY_ATTEMPTS} attempts: {error}"
        )))
    }

    async fn recover_missing_answer(&self) -> anyhow::Result<TurnStatus> {
        let mut last_error = None;
        for attempt in 1..=MAX_RECOVERY_ATTEMPTS {
            let prompt = missing_answer_recovery_prompt(attempt, MAX_RECOVERY_ATTEMPTS);
            match self.run_prompt(prompt).await? {
                StreamRunOutcome::Completed(completion) if completion.has_output() => {
                    return Ok(TurnStatus::Complete(completion.output().to_string()));
                }
                StreamRunOutcome::Completed(_) => last_error = None,
                StreamRunOutcome::Failed(error) => last_error = Some(error),
            }
        }

        Ok(TurnStatus::RecoveryStopped(missing_answer_error(
            last_error,
        )))
    }

    async fn recover_unresolved_tool(&self) -> anyhow::Result<TurnStatus> {
        let mut last_error = None;
        for attempt in 1..=MAX_TOOL_RECOVERY_ATTEMPTS {
            let prompt = unresolved_tool_recovery_prompt(attempt, MAX_TOOL_RECOVERY_ATTEMPTS);
            match self.run_prompt(prompt).await? {
                StreamRunOutcome::Completed(completion) if completion.has_output() => {
                    return Ok(TurnStatus::Complete(completion.output().to_string()));
                }
                StreamRunOutcome::Completed(_) => last_error = None,
                StreamRunOutcome::Failed(error) => last_error = Some(error),
            }
        }

        Ok(TurnStatus::RecoveryStopped(unresolved_tool_error(
            last_error,
        )))
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
