use std::sync::Arc;

use futures::future::BoxFuture;

use crate::{
    entities::{
        conversation::{context_usage::ContextUsage, turn::TurnStatus},
        recovery::{RecoveryOutcome, RecoveryRequest},
    },
    infrastructure::{
        prompts::PromptProvider,
        recovery::{RecoveryTurn, TurnRecovery},
    },
};

const MAX_RECOVERY_ATTEMPTS: usize = 3;
const MAX_TOOL_RECOVERY_ATTEMPTS: usize = 2;

/// Applies retry, completion, and error-reporting policy for incomplete model turns.
pub(crate) struct TurnRecoveryWorkflow {
    prompts: Arc<dyn PromptProvider>,
}

impl TurnRecoveryWorkflow {
    /// Creates the provider-neutral workflow with injected model-facing prompt composition.
    pub(crate) fn new(prompts: Arc<dyn PromptProvider>) -> Self {
        Self { prompts }
    }

    async fn recover_request(
        &self,
        request: RecoveryRequest,
        runner: &dyn RecoveryTurn,
        context_usage: ContextUsage,
    ) -> anyhow::Result<TurnStatus> {
        match request {
            RecoveryRequest::StreamFailure(error) => {
                self.recover_stream_failure(error, runner, context_usage)
                    .await
            }
            request @ RecoveryRequest::MissingAnswer
            | request @ RecoveryRequest::UnresolvedTool => {
                self.recover_answer(request, runner, context_usage).await
            }
        }
    }

    async fn recover_stream_failure(
        &self,
        mut error: String,
        runner: &dyn RecoveryTurn,
        mut context_usage: ContextUsage,
    ) -> anyhow::Result<TurnStatus> {
        for attempt in 1..=MAX_RECOVERY_ATTEMPTS {
            let prompt = self
                .prompts
                .recovery(&error, attempt, MAX_RECOVERY_ATTEMPTS);
            let outcome = runner.run_prompt(prompt).await?;
            if let Some(status) = Self::stream_outcome(outcome, &mut error, &mut context_usage) {
                return Ok(status);
            }
        }
        Ok(TurnStatus::RecoveryStopped {
            error: format!(
                "model failed to recover after {MAX_RECOVERY_ATTEMPTS} attempts: {error}"
            ),
            context_usage,
        })
    }

    async fn recover_answer(
        &self,
        request: RecoveryRequest,
        runner: &dyn RecoveryTurn,
        mut context_usage: ContextUsage,
    ) -> anyhow::Result<TurnStatus> {
        let max_attempts = answer_attempt_limit(&request);
        let mut last_error = None;
        for attempt in 1..=max_attempts {
            let prompt = self.answer_prompt(&request, attempt, max_attempts);
            let outcome = runner.run_prompt(prompt).await?;
            if let Some(status) = Self::answer_outcome(outcome, &mut context_usage, &mut last_error)
            {
                return Ok(status);
            }
        }
        Ok(TurnStatus::RecoveryStopped {
            error: answer_error(&request, last_error),
            context_usage,
        })
    }

    fn answer_prompt(&self, request: &RecoveryRequest, attempt: usize, max: usize) -> String {
        match request {
            RecoveryRequest::MissingAnswer => self.prompts.missing_answer_recovery(attempt, max),
            RecoveryRequest::UnresolvedTool => self.prompts.unresolved_tool_recovery(attempt, max),
            RecoveryRequest::StreamFailure(error) => self.prompts.recovery(error, attempt, max),
        }
    }

    fn answer_outcome(
        outcome: RecoveryOutcome,
        context_usage: &mut ContextUsage,
        last_error: &mut Option<String>,
    ) -> Option<TurnStatus> {
        if outcome.has_output() {
            return outcome.into_completed_status().ok();
        }
        match outcome.into_completed_status() {
            Ok(status) => {
                *context_usage = status.context_usage();
                *last_error = None;
            }
            Err((error, latest)) => {
                *context_usage = latest;
                *last_error = Some(error);
            }
        }
        None
    }

    fn stream_outcome(
        outcome: RecoveryOutcome,
        error: &mut String,
        context_usage: &mut ContextUsage,
    ) -> Option<TurnStatus> {
        match outcome.into_completed_status() {
            Ok(status) => Some(status),
            Err((next_error, latest_usage)) => {
                *error = next_error;
                *context_usage = latest_usage;
                None
            }
        }
    }
}

impl TurnRecovery for TurnRecoveryWorkflow {
    fn recover<'a>(
        &'a self,
        request: RecoveryRequest,
        runner: &'a dyn RecoveryTurn,
        context_usage: ContextUsage,
    ) -> BoxFuture<'a, anyhow::Result<TurnStatus>> {
        Box::pin(self.recover_request(request, runner, context_usage))
    }
}

fn answer_attempt_limit(request: &RecoveryRequest) -> usize {
    match request {
        RecoveryRequest::UnresolvedTool => MAX_TOOL_RECOVERY_ATTEMPTS,
        RecoveryRequest::MissingAnswer | RecoveryRequest::StreamFailure(_) => MAX_RECOVERY_ATTEMPTS,
    }
}

fn answer_error(request: &RecoveryRequest, last_error: Option<String>) -> String {
    match request {
        RecoveryRequest::MissingAnswer => missing_answer_error(last_error),
        RecoveryRequest::UnresolvedTool => unresolved_tool_error(last_error),
        RecoveryRequest::StreamFailure(error) => {
            format!("model failed to recover after {MAX_RECOVERY_ATTEMPTS} attempts: {error}")
        }
    }
}

fn missing_answer_error(last_error: Option<String>) -> String {
    match last_error {
        Some(error) => format!(
            "model failed to produce an answer after {MAX_RECOVERY_ATTEMPTS} recovery attempts: {error}"
        ),
        None => format!("model produced no answer after {MAX_RECOVERY_ATTEMPTS} recovery attempts"),
    }
}

fn unresolved_tool_error(last_error: Option<String>) -> String {
    match last_error {
        Some(error) => format!(
            "model failed to correct a tool error after {MAX_TOOL_RECOVERY_ATTEMPTS} recovery attempts: {error}"
        ),
        None => format!(
            "model produced no answer after correcting a tool error in {MAX_TOOL_RECOVERY_ATTEMPTS} recovery attempts"
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Mutex};

    use super::*;

    struct TestPrompts;

    impl PromptProvider for TestPrompts {
        fn chat_system(&self) -> BoxFuture<'_, anyhow::Result<String>> {
            Box::pin(async { Ok(String::new()) })
        }
        fn subagent_system(&self) -> BoxFuture<'_, anyhow::Result<String>> {
            Box::pin(async { Ok(String::new()) })
        }
        fn task(&self, task: &str) -> String {
            task.to_string()
        }
        fn summarization(&self) -> String {
            String::new()
        }
        fn goal_follow_up(&self, goal: &str) -> String {
            goal.to_string()
        }
        fn recovery(&self, error: &str, attempt: usize, max: usize) -> String {
            format!("stream:{error}:{attempt}/{max}")
        }
        fn missing_answer_recovery(&self, attempt: usize, max: usize) -> String {
            format!("missing:{attempt}/{max}")
        }
        fn unresolved_tool_recovery(&self, attempt: usize, max: usize) -> String {
            format!("tool:{attempt}/{max}")
        }
    }

    #[derive(Default)]
    struct FakeTurn {
        outcomes: Mutex<VecDeque<RecoveryOutcome>>,
        prompts: Mutex<Vec<String>>,
    }

    impl RecoveryTurn for FakeTurn {
        fn run_prompt(&self, prompt: String) -> BoxFuture<'_, anyhow::Result<RecoveryOutcome>> {
            self.prompts
                .lock()
                .map(|mut prompts| prompts.push(prompt))
                .unwrap_or_default();
            let outcome = self
                .outcomes
                .lock()
                .ok()
                .and_then(|mut outcomes| outcomes.pop_front());
            Box::pin(async move {
                outcome.ok_or_else(|| anyhow::anyhow!("test outcome queue was empty"))
            })
        }
    }

    fn usage(tokens: u64) -> ContextUsage {
        ContextUsage::new(Some(tokens), Some(1_000))
    }

    fn workflow() -> TurnRecoveryWorkflow {
        TurnRecoveryWorkflow::new(Arc::new(TestPrompts))
    }

    #[tokio::test]
    async fn stream_failure_retries_three_times_and_accepts_empty_completion() -> anyhow::Result<()>
    {
        let runner = FakeTurn::default();
        runner
            .outcomes
            .lock()
            .map_err(|_| anyhow::anyhow!("poisoned"))?
            .extend([
                RecoveryOutcome::Failed {
                    error: "first".to_string(),
                    context_usage: usage(10),
                },
                RecoveryOutcome::Failed {
                    error: "second".to_string(),
                    context_usage: usage(20),
                },
                RecoveryOutcome::Completed {
                    output: String::new(),
                    has_output: false,
                    context_usage: usage(30),
                },
            ]);
        let status = workflow()
            .recover(
                RecoveryRequest::StreamFailure("initial".to_string()),
                &runner,
                usage(1),
            )
            .await?;
        assert_eq!(
            status,
            TurnStatus::Complete {
                output: String::new(),
                context_usage: usage(30)
            }
        );
        assert_eq!(
            runner
                .prompts
                .lock()
                .map_err(|_| anyhow::anyhow!("poisoned"))?
                .as_slice(),
            [
                "stream:initial:1/3",
                "stream:first:2/3",
                "stream:second:3/3"
            ]
        );
        Ok(())
    }

    #[tokio::test]
    async fn missing_answer_requires_output_and_propagates_latest_usage() -> anyhow::Result<()> {
        let runner = FakeTurn::default();
        runner
            .outcomes
            .lock()
            .map_err(|_| anyhow::anyhow!("poisoned"))?
            .extend([
                RecoveryOutcome::Completed {
                    output: String::new(),
                    has_output: false,
                    context_usage: usage(10),
                },
                RecoveryOutcome::Failed {
                    error: "still failed".to_string(),
                    context_usage: usage(20),
                },
                RecoveryOutcome::Completed {
                    output: "answer".to_string(),
                    has_output: true,
                    context_usage: usage(30),
                },
            ]);
        let status = workflow()
            .recover(RecoveryRequest::MissingAnswer, &runner, usage(1))
            .await?;
        assert_eq!(
            status,
            TurnStatus::Complete {
                output: "answer".to_string(),
                context_usage: usage(30)
            }
        );
        assert_eq!(
            runner
                .prompts
                .lock()
                .map_err(|_| anyhow::anyhow!("poisoned"))?
                .len(),
            3
        );
        Ok(())
    }

    #[tokio::test]
    async fn unresolved_tool_stops_after_two_empty_results_with_latest_usage() -> anyhow::Result<()>
    {
        let runner = FakeTurn::default();
        runner
            .outcomes
            .lock()
            .map_err(|_| anyhow::anyhow!("poisoned"))?
            .extend([
                RecoveryOutcome::Completed {
                    output: String::new(),
                    has_output: false,
                    context_usage: usage(10),
                },
                RecoveryOutcome::Completed {
                    output: String::new(),
                    has_output: false,
                    context_usage: usage(20),
                },
            ]);
        let status = workflow()
            .recover(RecoveryRequest::UnresolvedTool, &runner, usage(1))
            .await?;
        assert_eq!(
            status,
            TurnStatus::RecoveryStopped {
                error:
                    "model produced no answer after correcting a tool error in 2 recovery attempts"
                        .to_string(),
                context_usage: usage(20),
            }
        );
        assert_eq!(
            runner
                .prompts
                .lock()
                .map_err(|_| anyhow::anyhow!("poisoned"))?
                .as_slice(),
            ["tool:1/2", "tool:2/2"]
        );
        Ok(())
    }
}
