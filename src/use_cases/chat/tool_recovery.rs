use rig::{
    agent::{
        AgentHook, CompletionCallAction, CompletionCallEvent, HookContext, InvalidToolCallAction,
        InvalidToolCallContext, ModelTurnAction, ModelTurnFinished, RequestPatch, ToolResultAction,
        ToolResultEvent,
    },
    message::{AssistantContent, ToolChoice, ToolResult as MessageToolResult, ToolResultContent},
    tool::ToolOutput,
};

const MAX_EMPTY_RECOVERY_ATTEMPTS: usize = 2;
const MAX_TOTAL_TOOL_FAILURES: usize = 6;
const MAX_REPEATED_FAILURES: usize = 2;
pub(crate) const MAX_INVALID_TOOL_CALL_ATTEMPTS: usize = 2;
const TOOL_STATUS_FIELD: &str = "rigel_tool_status";
const TOOL_STATUS_ERROR: &str = "error";
const TOOL_STATUS_RECOVERED: &str = "recovered";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ToolRecoveryStatus {
    None,
    Error,
    Recovered,
}

#[derive(Default)]
pub(crate) struct ToolRecoveryHook;

#[derive(Clone, Debug)]
struct PendingToolFailure {
    tool_name: String,
    diagnostic: String,
    failed_turn: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ToolFailureFingerprint {
    tool_name: String,
    args: String,
    diagnostic: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ToolInvocationFingerprint {
    tool_name: String,
    args: String,
}

#[derive(Clone, Debug, Default)]
struct ToolRecoveryState {
    pending: Option<PendingToolFailure>,
    empty_recovery_attempts: usize,
    total_failures: usize,
    invalid_tool_call_attempts: usize,
    failure_history: Vec<ToolFailureFingerprint>,
    invocation_history: Vec<ToolInvocationFingerprint>,
    force_final_answer: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
enum ToolResultDecision {
    Keep,
    ReportFailure {
        total_failures: usize,
        force_final_answer: Option<String>,
    },
    ReportRecovery,
}

#[derive(Debug, PartialEq, Eq)]
enum ModelTurnDecision {
    Continue,
    Retry { prompt: String },
    Stop { message: String },
}

impl AgentHook for ToolRecoveryHook {
    async fn on_completion_call(
        &self,
        ctx: &HookContext,
        _event: CompletionCallEvent<'_>,
    ) -> CompletionCallAction {
        let force_final_answer = ctx
            .scratchpad()
            .get::<ToolRecoveryState>()
            .is_some_and(|state| state.force_final_answer.is_some());

        if force_final_answer {
            return CompletionCallAction::patch(RequestPatch::new().tool_choice(ToolChoice::None));
        }

        CompletionCallAction::continue_run()
    }

    async fn on_tool_result(
        &self,
        ctx: &HookContext,
        event: ToolResultEvent<'_>,
    ) -> ToolResultAction {
        let failed = event.raw_result.is_error() || event.raw_result.is_refused();
        let successful = event.raw_result.is_success();
        let no_op = is_no_op_result(event.presentation);
        let diagnostic = event.presentation.render();
        let decision = ctx.scratchpad().update::<ToolRecoveryState, _>(|state| {
            state.observe_tool_result(
                event.tool_name,
                event.args,
                ctx.turn(),
                failed,
                successful,
                no_op,
                diagnostic.as_str(),
            )
        });

        match decision {
            ToolResultDecision::Keep => ToolResultAction::keep(),
            ToolResultDecision::ReportFailure {
                total_failures,
                force_final_answer,
            } => ToolResultAction::rewrite_output(append_status_marker(
                event.presentation,
                serde_json::json!({
                    TOOL_STATUS_FIELD: TOOL_STATUS_ERROR,
                    "tool": event.tool_name,
                    "next_action": tool_error_next_action(
                        event.tool_name,
                        force_final_answer.as_deref()
                    ),
                    "total_failures": total_failures
                }),
            )),
            ToolResultDecision::ReportRecovery => {
                ToolResultAction::rewrite_output(append_status_marker(
                    event.presentation,
                    serde_json::json!({
                        TOOL_STATUS_FIELD: TOOL_STATUS_RECOVERED,
                        "tool": event.tool_name
                    }),
                ))
            }
        }
    }

    async fn on_model_turn_finished(
        &self,
        ctx: &HookContext,
        event: ModelTurnFinished<'_>,
    ) -> ModelTurnAction {
        let has_tool_call = event
            .content
            .iter()
            .any(|content| matches!(content, AssistantContent::ToolCall(_)));
        let has_answer = event.content.iter().any(|content| {
            matches!(content, AssistantContent::Text(text) if !text.text().trim().is_empty())
        });
        let decision = ctx.scratchpad().update::<ToolRecoveryState, _>(|state| {
            state.decide_model_turn(has_tool_call, has_answer)
        });

        match decision {
            ModelTurnDecision::Continue => ModelTurnAction::continue_run(),
            ModelTurnDecision::Retry { prompt } => ModelTurnAction::retry_with_feedback(prompt),
            ModelTurnDecision::Stop { message } => ModelTurnAction::stop(message),
        }
    }

    async fn on_invalid_tool_call(
        &self,
        ctx: &HookContext,
        event: &InvalidToolCallContext,
    ) -> Option<InvalidToolCallAction> {
        let (attempt, force_final_answer) =
            ctx.scratchpad().update::<ToolRecoveryState, _>(|state| {
                state.invalid_tool_call_attempts += 1;
                (
                    state.invalid_tool_call_attempts,
                    state.force_final_answer.clone(),
                )
            });

        if attempt <= MAX_INVALID_TOOL_CALL_ATTEMPTS {
            if let Some(reason) = force_final_answer {
                return Some(InvalidToolCallAction::retry(format!(
                    "Tools are disabled for this final recovery response because {reason}. Do not \
                     emit another tool call. Return a non-empty final answer that briefly states \
                     what succeeded and what could not be completed. Recovery attempt \
                     {attempt}/{MAX_INVALID_TOOL_CALL_ATTEMPTS}."
                )));
            }

            let available_tools = if event.available_tools.is_empty() {
                "<none>".to_string()
            } else {
                event.available_tools.join(", ")
            };
            return Some(InvalidToolCallAction::retry(format!(
                "The \"{}\" tool call could not be dispatched. Available tools: \
                 {available_tools}. If a tool is still needed, correct its name and arguments and \
                 call it. If the call was unnecessary or the user request is already complete, \
                 return a non-empty final answer instead. Do not call an unrelated tool. Recovery \
                 attempt {attempt}/{MAX_INVALID_TOOL_CALL_ATTEMPTS}.",
                event.tool_name
            )));
        }

        Some(InvalidToolCallAction::stop(format!(
            "model emitted an invalid tool call more than {MAX_INVALID_TOOL_CALL_ATTEMPTS} times; \
             last tool name was \"{}\"",
            event.tool_name
        )))
    }
}

impl ToolRecoveryState {
    #[allow(clippy::too_many_arguments)]
    fn observe_tool_result(
        &mut self,
        tool_name: &str,
        args: &str,
        turn: usize,
        failed: bool,
        successful: bool,
        no_op: bool,
        diagnostic: &str,
    ) -> ToolResultDecision {
        self.invocation_history.push(ToolInvocationFingerprint {
            tool_name: tool_name.to_string(),
            args: args.to_string(),
        });

        if failed {
            self.total_failures += 1;
            self.empty_recovery_attempts = 0;
            self.pending = Some(PendingToolFailure {
                tool_name: tool_name.to_string(),
                diagnostic: diagnostic.to_string(),
                failed_turn: turn,
            });

            let fingerprint = ToolFailureFingerprint {
                tool_name: tool_name.to_string(),
                args: args.to_string(),
                diagnostic: diagnostic.to_string(),
            };
            self.failure_history.push(fingerprint.clone());

            let repeated = self
                .failure_history
                .iter()
                .filter(|previous| **previous == fingerprint)
                .count();
            self.force_final_answer = force_final_reason(
                self.total_failures,
                repeated,
                self.invocation_history.as_slice(),
            );

            return ToolResultDecision::ReportFailure {
                total_failures: self.total_failures,
                force_final_answer: self.force_final_answer.clone(),
            };
        }

        if self.pending.is_some()
            && self.force_final_answer.is_none()
            && has_short_invocation_cycle(self.invocation_history.as_slice())
        {
            let reason = "tool calls entered a repeated A-B-A-B cycle".to_string();
            self.force_final_answer = Some(reason.clone());
            return ToolResultDecision::ReportFailure {
                total_failures: self.total_failures,
                force_final_answer: Some(reason),
            };
        }

        let recovered = successful
            && !no_op
            && self.pending.as_ref().is_some_and(|failure| {
                turn > failure.failed_turn && tool_name == failure.tool_name
            });

        if recovered {
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

        self.empty_recovery_attempts += 1;

        if self.empty_recovery_attempts <= MAX_EMPTY_RECOVERY_ATTEMPTS {
            return ModelTurnDecision::Retry {
                prompt: tool_retry_prompt(
                    &failure,
                    self.empty_recovery_attempts,
                    MAX_EMPTY_RECOVERY_ATTEMPTS,
                ),
            };
        }

        ModelTurnDecision::Stop {
            message: format!(
                "model produced neither a corrective tool call nor a final answer after {} \
                 recovery attempts; last \"{}\" diagnostic: {}",
                MAX_EMPTY_RECOVERY_ATTEMPTS, failure.tool_name, failure.diagnostic
            ),
        }
    }
}

pub(crate) fn tool_recovery_status(tool_result: &MessageToolResult) -> ToolRecoveryStatus {
    tool_result
        .content
        .iter()
        .filter_map(ToolResultContent::as_json)
        .find_map(|value| {
            match value
                .get(TOOL_STATUS_FIELD)
                .and_then(|value| value.as_str())
            {
                Some(TOOL_STATUS_ERROR) => Some(ToolRecoveryStatus::Error),
                Some(TOOL_STATUS_RECOVERED) => Some(ToolRecoveryStatus::Recovered),
                _ => None,
            }
        })
        .unwrap_or(ToolRecoveryStatus::None)
}

fn force_final_reason(
    total_failures: usize,
    repeated: usize,
    invocation_history: &[ToolInvocationFingerprint],
) -> Option<String> {
    if repeated > MAX_REPEATED_FAILURES {
        return Some(format!(
            "the same tool call failed more than {MAX_REPEATED_FAILURES} times"
        ));
    }

    if has_short_invocation_cycle(invocation_history) {
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

fn is_no_op_result(output: &ToolOutput) -> bool {
    output
        .as_content()
        .iter()
        .filter_map(ToolResultContent::as_json)
        .any(|value| value.get("action").and_then(serde_json::Value::as_str) == Some("not_found"))
}

fn append_status_marker(output: &ToolOutput, marker: serde_json::Value) -> ToolOutput {
    let mut content = output.as_content().clone();
    content.push(ToolResultContent::json(marker));
    ToolOutput::content(content)
}

fn tool_error_next_action(tool_name: &str, force_final_reason: Option<&str>) -> String {
    if let Some(reason) = force_final_reason {
        return format!(
            "Do not call another tool because {reason}. Return a non-empty final answer that \
             briefly states what succeeded and what could not be completed."
        );
    }

    format!(
        "The {tool_name} call failed. If this operation is still required, correct the tool choice \
         or arguments and try again. If the call was unnecessary or the request is already \
         complete, return a non-empty final answer. Do not call an unrelated tool."
    )
}

fn tool_retry_prompt(failure: &PendingToolFailure, attempt: usize, max_attempts: usize) -> String {
    format!(
        "The previous \"{}\" tool call failed. Diagnostic:\n\
         <tool_error>{}</tool_error>\n\
         Respond now with exactly one useful next step: call a corrected tool if the failed \
         operation is still needed, or return a non-empty final answer if the call was unnecessary \
         or the user request is already complete. Do not call an unrelated tool. Recovery attempt \
         {attempt}/{max_attempts}.",
        failure.tool_name, failure.diagnostic
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::{OneOrMany, message::ToolResult as MessageToolResult};

    fn failure(turn: usize) -> ToolRecoveryState {
        let mut state = ToolRecoveryState::default();
        let decision = state.observe_tool_result(
            "read_file",
            r#"{"path":"missing.rs"}"#,
            turn,
            true,
            false,
            false,
            r#"{"code":"PATH_NOT_FOUND"}"#,
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

    fn record_failure(state: &mut ToolRecoveryState, tool: &str, args: &str, turn: usize) {
        state.observe_tool_result(tool, args, turn, true, false, false, r#"{"code":"FAILED"}"#);
    }

    #[test]
    fn final_answer_after_failure_is_accepted() {
        let mut state = failure(1);

        assert_eq!(
            state.decide_model_turn(false, true),
            ModelTurnDecision::Continue
        );
        assert!(state.pending.is_none());
    }

    #[test]
    fn empty_turn_after_failure_is_retried() {
        let mut state = failure(1);

        let ModelTurnDecision::Retry { prompt } = state.decide_model_turn(false, false) else {
            panic!("empty response must be retried");
        };
        assert!(prompt.contains("PATH_NOT_FOUND"));
        assert!(prompt.contains("or return a non-empty final answer"));
    }

    #[test]
    fn corrected_tool_call_is_allowed_to_execute() {
        let mut state = failure(1);

        assert_eq!(
            state.decide_model_turn(true, false),
            ModelTurnDecision::Continue
        );
    }

    #[test]
    fn unrelated_success_does_not_resolve_failure() {
        let mut state = failure(2);

        assert_eq!(
            state.observe_tool_result("run_command", "{}", 3, false, true, false, "ok"),
            ToolResultDecision::Keep
        );
        assert!(state.pending.is_some());
    }

    #[test]
    fn same_tool_success_on_later_turn_resolves_failure() {
        let mut state = failure(2);

        assert_eq!(
            state.observe_tool_result(
                "read_file",
                r#"{"path":"src/main.rs"}"#,
                3,
                false,
                true,
                false,
                "ok",
            ),
            ToolResultDecision::ReportRecovery
        );
        assert!(state.pending.is_none());
    }

    #[test]
    fn not_found_no_op_does_not_resolve_failure() {
        let mut state = ToolRecoveryState::default();
        record_failure(&mut state, "delete_path", r#"{"path":"."}"#, 1);

        assert_eq!(
            state.observe_tool_result(
                "delete_path",
                r#"{"path":"asd"}"#,
                2,
                false,
                true,
                true,
                r#"{"action":"not_found","path":"asd","kind":"unknown"}"#,
            ),
            ToolResultDecision::Keep
        );
        assert!(state.pending.is_some());
    }

    #[test]
    fn repeated_identical_failure_forces_final_answer() {
        let mut state = ToolRecoveryState::default();

        for turn in 1..=3 {
            record_failure(&mut state, "delete_path", r#"{"path":"."}"#, turn);
        }

        assert!(state.force_final_answer.is_some());
    }

    #[test]
    fn alternating_failure_cycle_forces_final_answer() {
        let mut state = ToolRecoveryState::default();
        record_failure(&mut state, "delete_path", r#"{"path":"."}"#, 1);
        record_failure(&mut state, "delete_path", r#"{"path":"asd"}"#, 2);
        record_failure(&mut state, "delete_path", r#"{"path":"."}"#, 3);
        record_failure(&mut state, "delete_path", r#"{"path":"asd"}"#, 4);

        assert_eq!(
            state.force_final_answer.as_deref(),
            Some("tool calls entered a repeated A-B-A-B cycle")
        );
    }

    #[test]
    fn error_and_no_op_cycle_from_regression_forces_final_answer() {
        let mut state = ToolRecoveryState::default();
        record_failure(&mut state, "delete_path", r#"{"path":"."}"#, 1);
        state.observe_tool_result(
            "delete_path",
            r#"{"path":"asd"}"#,
            2,
            false,
            true,
            true,
            r#"{"action":"not_found","path":"asd","kind":"unknown"}"#,
        );
        record_failure(&mut state, "delete_path", r#"{"path":"."}"#, 3);

        let decision = state.observe_tool_result(
            "delete_path",
            r#"{"path":"asd"}"#,
            4,
            false,
            true,
            true,
            r#"{"action":"not_found","path":"asd","kind":"unknown"}"#,
        );

        assert_eq!(
            decision,
            ToolResultDecision::ReportFailure {
                total_failures: 2,
                force_final_answer: Some("tool calls entered a repeated A-B-A-B cycle".to_string()),
            }
        );
    }

    #[test]
    fn total_failure_budget_is_not_reset_by_success() {
        let mut state = ToolRecoveryState::default();

        for turn in 1..MAX_TOTAL_TOOL_FAILURES {
            record_failure(
                &mut state,
                format!("tool_{turn}").as_str(),
                format!(r#"{{"attempt":{turn}}}"#).as_str(),
                turn,
            );
            state.observe_tool_result(
                format!("tool_{turn}").as_str(),
                "{}",
                turn + 1,
                false,
                true,
                false,
                "ok",
            );
        }
        record_failure(
            &mut state,
            "last_tool",
            r#"{"attempt":"last"}"#,
            MAX_TOTAL_TOOL_FAILURES * 2,
        );

        assert!(state.force_final_answer.is_some());
        assert_eq!(state.total_failures, MAX_TOTAL_TOOL_FAILURES);
    }

    #[test]
    fn repeated_empty_answers_eventually_stop() {
        let mut state = failure(1);

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
    fn status_marker_is_detected_without_io() {
        let result = MessageToolResult {
            id: "call-1".to_string(),
            call_id: None,
            content: OneOrMany::one(ToolResultContent::json(serde_json::json!({
                TOOL_STATUS_FIELD: TOOL_STATUS_ERROR
            }))),
        };

        assert_eq!(tool_recovery_status(&result), ToolRecoveryStatus::Error);
    }
}
