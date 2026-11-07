use crate::{
    entities::recovery::{
        InvalidToolCallDecision, MAX_INVALID_TOOL_CALL_ATTEMPTS, ModelTurnDecision,
        PendingToolFailure, ToolFailureFingerprint, ToolInvocationFingerprint, ToolResultDecision,
        ToolResultObservation,
    },
    infrastructure::recovery::{
        ToolRecoveryPolicy, ToolRecoveryPolicyFactory as ToolRecoveryPolicyFactoryPort,
    },
};

const MAX_EMPTY_RECOVERY_ATTEMPTS: usize = 2;
const MAX_TOTAL_TOOL_FAILURES: usize = 6;
const MAX_REPEATED_FAILURES: usize = 2;

/// Tracks pending failures, retry budgets, and invocation history for one agent run.
#[derive(Clone, Default)]
pub(crate) struct ToolRecoveryState {
    pending: Option<PendingToolFailure>,
    empty_recovery_attempts: usize,
    total_failures: usize,
    invalid_tool_call_attempts: usize,
    failure_history: Vec<ToolFailureFingerprint>,
    invocation_history: Vec<ToolInvocationFingerprint>,
    force_final_answer: Option<String>,
}

/// Creates a fresh tool recovery policy state for each run.
#[derive(Default)]
pub(crate) struct DefaultToolRecoveryPolicyFactory;

impl DefaultToolRecoveryPolicyFactory {
    /// Creates a stateless factory; each product owns fresh recovery counters.
    pub(crate) fn new() -> Self {
        Self
    }
}

impl ToolRecoveryPolicyFactoryPort for DefaultToolRecoveryPolicyFactory {
    fn create(&self) -> Box<dyn ToolRecoveryPolicy> {
        Box::new(ToolRecoveryState::default())
    }
}

impl ToolRecoveryPolicy for ToolRecoveryState {
    fn clone_box(&self) -> Box<dyn ToolRecoveryPolicy> {
        Box::new(self.clone())
    }

    fn observe(&mut self, observation: ToolResultObservation) -> ToolResultDecision {
        self.invocation_history.push(ToolInvocationFingerprint {
            tool_name: observation.tool_name.clone(),
            args: observation.args.clone(),
        });
        if observation.failed {
            return self.record_failure(&observation);
        }
        if self.should_force_final_answer() {
            return self.failure_decision();
        }
        if self.is_recovered(&observation) {
            self.pending = None;
            self.empty_recovery_attempts = 0;
            return ToolResultDecision::ReportRecovery;
        }
        ToolResultDecision::Keep
    }

    fn decide_model_turn(&mut self, has_tool_call: bool, has_answer: bool) -> ModelTurnDecision {
        let Some(failure) = self.pending.clone() else {
            return ModelTurnDecision::Continue;
        };
        if has_tool_call {
            return ModelTurnDecision::Continue;
        }
        if has_answer {
            self.pending = None;
            self.empty_recovery_attempts = 0;
            return ModelTurnDecision::Continue;
        }
        self.retry_or_stop(failure)
    }

    fn invalid_call(&mut self) -> InvalidToolCallDecision {
        self.invalid_tool_call_attempts += 1;
        if self.invalid_tool_call_attempts <= MAX_INVALID_TOOL_CALL_ATTEMPTS {
            return InvalidToolCallDecision::Retry {
                attempt: self.invalid_tool_call_attempts,
                max_attempts: MAX_INVALID_TOOL_CALL_ATTEMPTS,
                force_final_answer: self.force_final_answer.clone(),
            };
        }
        InvalidToolCallDecision::Stop {
            attempt: self.invalid_tool_call_attempts,
            max_attempts: MAX_INVALID_TOOL_CALL_ATTEMPTS,
        }
    }

    fn force_final(&self) -> Option<String> {
        self.force_final_answer.clone()
    }
}

impl ToolRecoveryState {
    fn record_failure(&mut self, observation: &ToolResultObservation) -> ToolResultDecision {
        self.total_failures += 1;
        self.empty_recovery_attempts = 0;
        self.pending = Some(PendingToolFailure {
            tool_name: observation.tool_name.clone(),
            diagnostic: observation.diagnostic.clone(),
            failed_turn: observation.turn,
        });
        self.record_fingerprint(observation);
        self.failure_decision()
    }

    fn record_fingerprint(&mut self, observation: &ToolResultObservation) {
        let fingerprint = ToolFailureFingerprint {
            tool_name: observation.tool_name.clone(),
            args: observation.args.clone(),
            diagnostic: observation.diagnostic.clone(),
        };
        self.failure_history.push(fingerprint.clone());
        let repeated = self
            .failure_history
            .iter()
            .filter(|item| **item == fingerprint)
            .count();
        self.force_final_answer =
            force_final_reason(self.total_failures, repeated, &self.invocation_history);
    }

    fn failure_decision(&self) -> ToolResultDecision {
        ToolResultDecision::ReportFailure {
            total_failures: self.total_failures,
            force_final_answer: self.force_final_answer.clone(),
        }
    }

    fn is_recovered(&self, observation: &ToolResultObservation) -> bool {
        observation.successful
            && !observation.no_op
            && self.pending.as_ref().is_some_and(|failure| {
                observation.turn > failure.failed_turn && observation.tool_name == failure.tool_name
            })
    }

    fn should_force_final_answer(&mut self) -> bool {
        if self.pending.is_some()
            && self.force_final_answer.is_none()
            && has_short_invocation_cycle(&self.invocation_history)
        {
            self.force_final_answer =
                Some("tool calls entered a repeated A-B-A-B cycle".to_string());
        }
        self.force_final_answer.is_some()
    }

    fn retry_or_stop(&mut self, failure: PendingToolFailure) -> ModelTurnDecision {
        self.empty_recovery_attempts += 1;
        if self.empty_recovery_attempts <= MAX_EMPTY_RECOVERY_ATTEMPTS {
            return ModelTurnDecision::Retry {
                prompt: tool_retry_prompt(&failure, self.empty_recovery_attempts),
            };
        }
        ModelTurnDecision::Stop {
            message: format!(
                "model produced neither a corrective tool call nor a final answer after {} recovery attempts; last \"{}\" diagnostic: {}",
                MAX_EMPTY_RECOVERY_ATTEMPTS, failure.tool_name, failure.diagnostic
            ),
        }
    }
}

fn force_final_reason(
    total_failures: usize,
    repeated: usize,
    history: &[ToolInvocationFingerprint],
) -> Option<String> {
    if repeated > MAX_REPEATED_FAILURES {
        return Some(format!(
            "the same tool call failed more than {MAX_REPEATED_FAILURES} times"
        ));
    }
    if has_short_invocation_cycle(history) {
        return Some("tool calls entered a repeated A-B-A-B cycle".to_string());
    }
    if total_failures >= MAX_TOTAL_TOOL_FAILURES {
        return Some(format!(
            "the request reached the limit of {MAX_TOTAL_TOOL_FAILURES} failed tool calls"
        ));
    }
    None
}

fn has_short_invocation_cycle(history: &[ToolInvocationFingerprint]) -> bool {
    let [.., first, second, third, fourth] = history else {
        return false;
    };
    first == third && second == fourth && first != second
}

fn tool_retry_prompt(failure: &PendingToolFailure, attempt: usize) -> String {
    let replacements = [
        ("{tool}", failure.tool_name.clone()),
        ("{diagnostic}", failure.diagnostic.clone()),
        ("{attempt}", attempt.to_string()),
        ("{max_attempts}", MAX_EMPTY_RECOVERY_ATTEMPTS.to_string()),
    ];
    render_template(
        include_str!("../../../prompts/recovery/tool_retry.md"),
        &replacements,
    )
}

fn render_template(template: &str, replacements: &[(&str, String)]) -> String {
    let mut remaining = template.trim_end_matches('\n');
    let mut rendered = String::with_capacity(remaining.len());
    while !remaining.is_empty() {
        if let Some((token, value)) = replacements
            .iter()
            .find(|(token, _)| remaining.starts_with(token))
        {
            rendered.push_str(value);
            remaining = &remaining[token.len()..];
            continue;
        }
        let Some(character) = remaining.chars().next() else {
            break;
        };
        rendered.push(character);
        remaining = &remaining[character.len_utf8()..];
    }
    rendered
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(tool: &str, args: &str, turn: usize) -> ToolResultObservation {
        ToolResultObservation::new(tool, args, turn)
    }

    fn failed_state(turn: usize) -> ToolRecoveryState {
        let mut state = ToolRecoveryState::default();
        let decision = state.observe(
            observation("test_tool", r#"{"path":"missing.rs"}"#, turn)
                .failed(r#"{"code":"PATH_NOT_FOUND"}"#),
        );
        assert_eq!(
            decision,
            ToolResultDecision::ReportFailure {
                total_failures: 1,
                force_final_answer: None,
            }
        );
        state
    }

    fn record_failure(state: &mut ToolRecoveryState, observation: ToolResultObservation) {
        state.observe(observation);
    }

    #[test]
    fn final_answer_after_failure_is_accepted() {
        let mut state = failed_state(1);
        assert_eq!(
            state.decide_model_turn(false, true),
            ModelTurnDecision::Continue
        );
        assert!(state.pending.is_none());
    }

    #[test]
    fn empty_turn_after_failure_is_retried() {
        let mut state = failed_state(1);
        let decision = state.decide_model_turn(false, false);
        assert!(matches!(decision, ModelTurnDecision::Retry { .. }));
        if let ModelTurnDecision::Retry { prompt } = decision {
            assert!(prompt.contains("PATH_NOT_FOUND"));
            assert!(prompt.contains("or return a non-empty final answer"));
        }
    }

    #[test]
    fn corrected_tool_call_is_allowed_to_execute() {
        let mut state = failed_state(1);
        assert_eq!(
            state.decide_model_turn(true, false),
            ModelTurnDecision::Continue
        );
    }

    #[test]
    fn unrelated_success_does_not_resolve_failure() {
        let mut state = failed_state(2);
        assert_eq!(
            state.observe(observation("run_command", "{}", 3).successful("ok")),
            ToolResultDecision::Keep
        );
        assert!(state.pending.is_some());
    }

    #[test]
    fn same_tool_success_on_later_turn_resolves_failure() {
        let mut state = failed_state(2);
        assert_eq!(
            state
                .observe(observation("test_tool", r#"{"path":"src/main.rs"}"#, 3).successful("ok")),
            ToolResultDecision::ReportRecovery
        );
        assert!(state.pending.is_none());
    }

    #[test]
    fn not_found_no_op_does_not_resolve_failure() {
        let mut state = ToolRecoveryState::default();
        record_failure(
            &mut state,
            observation("test_tool", r#"{"path":"."}"#, 1).failed(r#"{"code":"FAILED"}"#),
        );
        assert_eq!(
            state.observe(
                observation("test_tool", r#"{"path":"asd"}"#, 2)
                    .successful(r#"{"action":"not_found","path":"asd","kind":"unknown"}"#)
                    .no_op()
            ),
            ToolResultDecision::Keep
        );
        assert!(state.pending.is_some());
    }

    #[test]
    fn repeated_identical_failure_forces_final_answer() {
        let mut state = ToolRecoveryState::default();
        for turn in 1..=3 {
            record_failure(
                &mut state,
                observation("test_tool", r#"{"path":"."}"#, turn).failed(r#"{"code":"FAILED"}"#),
            );
        }
        assert!(state.force_final_answer.is_some());
    }

    #[test]
    fn alternating_failure_cycle_forces_final_answer() {
        let mut state = ToolRecoveryState::default();
        record_failure(
            &mut state,
            observation("test_tool", r#"{"path":"."}"#, 1).failed(r#"{"code":"FAILED"}"#),
        );
        record_failure(
            &mut state,
            observation("test_tool", r#"{"path":"asd"}"#, 2).failed(r#"{"code":"FAILED"}"#),
        );
        record_failure(
            &mut state,
            observation("test_tool", r#"{"path":"."}"#, 3).failed(r#"{"code":"FAILED"}"#),
        );
        record_failure(
            &mut state,
            observation("test_tool", r#"{"path":"asd"}"#, 4).failed(r#"{"code":"FAILED"}"#),
        );
        assert_eq!(
            state.force_final_answer.as_deref(),
            Some("tool calls entered a repeated A-B-A-B cycle")
        );
    }

    #[test]
    fn error_and_no_op_cycle_from_regression_forces_final_answer() {
        let mut state = ToolRecoveryState::default();
        record_failure(
            &mut state,
            observation("test_tool", r#"{"path":"."}"#, 1).failed(r#"{"code":"FAILED"}"#),
        );
        state.observe(
            observation("test_tool", r#"{"path":"asd"}"#, 2)
                .successful(r#"{"action":"not_found","path":"asd","kind":"unknown"}"#)
                .no_op(),
        );
        record_failure(
            &mut state,
            observation("test_tool", r#"{"path":"."}"#, 3).failed(r#"{"code":"FAILED"}"#),
        );
        let decision = state.observe(
            observation("test_tool", r#"{"path":"asd"}"#, 4)
                .successful(r#"{"action":"not_found","path":"asd","kind":"unknown"}"#)
                .no_op(),
        );
        assert_eq!(
            decision,
            ToolResultDecision::ReportFailure {
                total_failures: 2,
                force_final_answer: Some("tool calls entered a repeated A-B-A-B cycle".to_string())
            }
        );
    }

    #[test]
    fn total_failure_budget_is_not_reset_by_success() {
        let mut state = ToolRecoveryState::default();
        for turn in 1..MAX_TOTAL_TOOL_FAILURES {
            let tool = format!("tool_{turn}");
            record_failure(
                &mut state,
                observation(&tool, &format!(r#"{{"attempt":{turn}}}"#), turn)
                    .failed(r#"{"code":"FAILED"}"#),
            );
            state.observe(observation(&tool, "{}", turn + 1).successful("ok"));
        }
        record_failure(
            &mut state,
            observation(
                "last_tool",
                r#"{"attempt":"last"}"#,
                MAX_TOTAL_TOOL_FAILURES * 2,
            )
            .failed(r#"{"code":"FAILED"}"#),
        );
        assert!(state.force_final_answer.is_some());
        assert_eq!(state.total_failures, MAX_TOTAL_TOOL_FAILURES);
    }

    #[test]
    fn repeated_empty_answers_eventually_stop() {
        let mut state = failed_state(1);
        for _ in 0..MAX_EMPTY_RECOVERY_ATTEMPTS {
            assert!(matches!(
                state.decide_model_turn(false, false),
                ModelTurnDecision::Retry { .. }
            ));
        }
        assert!(matches!(
            state.decide_model_turn(false, false),
            ModelTurnDecision::Stop { .. }
        ));
    }

    #[test]
    fn forced_final_answer_is_present_in_invalid_call_retry() {
        let mut state = ToolRecoveryState::default();
        for turn in 1..=3 {
            record_failure(
                &mut state,
                observation("test_tool", r#"{"path":"."}"#, turn).failed(r#"{"code":"FAILED"}"#),
            );
        }
        assert!(matches!(
            state.invalid_call(),
            InvalidToolCallDecision::Retry {
                force_final_answer: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn invalid_call_budget_stops_after_two_retries() {
        let mut state = ToolRecoveryState::default();
        assert!(matches!(
            state.invalid_call(),
            InvalidToolCallDecision::Retry { attempt: 1, .. }
        ));
        assert!(matches!(
            state.invalid_call(),
            InvalidToolCallDecision::Retry { attempt: 2, .. }
        ));
        assert!(matches!(
            state.invalid_call(),
            InvalidToolCallDecision::Stop { attempt: 3, .. }
        ));
    }

    #[test]
    fn policy_factory_creates_isolated_state() {
        let factory = DefaultToolRecoveryPolicyFactory::new();
        let mut first = factory.create();
        let second = factory.create();
        first.observe(observation("test_tool", "{}", 1).failed("failed"));
        assert_eq!(second.force_final(), None);
    }
}
