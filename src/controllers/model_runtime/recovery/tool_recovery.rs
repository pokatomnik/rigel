use std::sync::Arc;

use rig::{
    agent::{
        AgentHook, CompletionCallAction, CompletionCallEvent, HookContext, InvalidToolCallAction,
        InvalidToolCallContext, ModelTurnAction, ModelTurnFinished, RequestPatch, ToolResultAction,
        ToolResultEvent,
    },
    message::{AssistantContent, ToolChoice, ToolResult as MessageToolResult, ToolResultContent},
    tool::ToolOutput,
};

use crate::{
    entities::recovery::{
        InvalidToolCallDecision, ModelTurnDecision, ToolResultDecision, ToolResultObservation,
    },
    infrastructure::recovery::ToolRecoveryPolicyFactory,
};

pub(crate) const MAX_INVALID_TOOL_CALL_ATTEMPTS: usize =
    crate::entities::recovery::MAX_INVALID_TOOL_CALL_ATTEMPTS;
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
struct RunPolicy {
    policy: Option<Box<dyn crate::infrastructure::recovery::ToolRecoveryPolicy>>,
}

impl Clone for RunPolicy {
    fn clone(&self) -> Self {
        Self {
            policy: self.policy.as_ref().map(|policy| policy.clone_box()),
        }
    }
}

pub(crate) struct ToolRecoveryHook {
    factory: Arc<dyn ToolRecoveryPolicyFactory>,
}

impl ToolRecoveryHook {
    /// Creates the provider hook with a factory for independent per-run policy state.
    pub(crate) fn new(factory: Arc<dyn ToolRecoveryPolicyFactory>) -> Self {
        Self { factory }
    }

    fn with_policy<T>(
        &self,
        ctx: &HookContext,
        operation: impl FnOnce(&mut dyn crate::infrastructure::recovery::ToolRecoveryPolicy) -> T,
    ) -> T {
        ctx.scratchpad().update::<RunPolicy, _>(|run| {
            let policy = run.policy.get_or_insert_with(|| self.factory.create());
            operation(policy.as_mut())
        })
    }
}

impl AgentHook for ToolRecoveryHook {
    async fn on_completion_call(
        &self,
        ctx: &HookContext,
        _event: CompletionCallEvent<'_>,
    ) -> CompletionCallAction {
        let force_final = ctx
            .scratchpad()
            .get::<RunPolicy>()
            .and_then(|run| run.policy)
            .and_then(|policy| policy.force_final());
        if force_final.is_some() {
            return CompletionCallAction::patch(RequestPatch::new().tool_choice(ToolChoice::None));
        }
        CompletionCallAction::continue_run()
    }

    async fn on_tool_result(
        &self,
        ctx: &HookContext,
        event: ToolResultEvent<'_>,
    ) -> ToolResultAction {
        let failed = event.raw_result.is_error()
            || event.raw_result.is_refused()
            || event.raw_result.is_skipped();
        let successful = event.raw_result.is_success();
        let no_op = is_no_op_result(event.presentation);
        let diagnostic = event.presentation.render();
        let observation = ToolResultObservation::new(event.tool_name, event.args, ctx.turn());
        let observation = if failed {
            observation.failed(diagnostic.as_str())
        } else if successful {
            observation.successful(diagnostic.as_str())
        } else {
            observation
        };
        let observation = if no_op {
            observation.no_op()
        } else {
            observation
        };
        let decision = self.with_policy(ctx, |policy| policy.observe(observation));
        self.tool_result_action(event.presentation, event.tool_name, decision)
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
        let has_answer = event.content.iter().any(|content| matches!(content, AssistantContent::Text(text) if !text.text.trim().is_empty()));
        let decision = self.with_policy(ctx, |policy| {
            policy.decide_model_turn(has_tool_call, has_answer)
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
        let decision = self.with_policy(ctx, |policy| policy.invalid_call());
        Some(invalid_tool_call_action(event, decision))
    }
}

impl ToolRecoveryHook {
    fn tool_result_action(
        &self,
        output: &ToolOutput,
        tool: &str,
        decision: ToolResultDecision,
    ) -> ToolResultAction {
        match decision {
            ToolResultDecision::Keep => ToolResultAction::keep(),
            ToolResultDecision::ReportFailure {
                total_failures,
                force_final_answer,
            } => {
                let marker = serde_json::json!({
                    TOOL_STATUS_FIELD: TOOL_STATUS_ERROR,
                    "tool": tool,
                    "next_action": tool_error_next_action(tool, force_final_answer.as_deref()),
                    "total_failures": total_failures
                });
                ToolResultAction::rewrite_output(append_status_marker(output, marker))
            }
            ToolResultDecision::ReportRecovery => {
                ToolResultAction::rewrite_output(append_status_marker(
                    output,
                    serde_json::json!({ TOOL_STATUS_FIELD: TOOL_STATUS_RECOVERED, "tool": tool }),
                ))
            }
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

fn invalid_tool_call_action(
    event: &InvalidToolCallContext,
    decision: InvalidToolCallDecision,
) -> InvalidToolCallAction {
    match decision {
        InvalidToolCallDecision::Retry {
            attempt,
            max_attempts,
            force_final_answer,
        } => {
            let template = invalid_tool_call_template(force_final_answer.as_deref());
            let message = render_template(
                template,
                &[
                    ("{reason}", force_final_answer.unwrap_or_default()),
                    ("{tool}", event.tool_name.clone()),
                    ("{available_tools}", available_tools(&event.available_tools)),
                    ("{attempt}", attempt.to_string()),
                    ("{max_attempts}", max_attempts.to_string()),
                ],
            );
            InvalidToolCallAction::retry(message)
        }
        InvalidToolCallDecision::Stop { max_attempts, .. } => InvalidToolCallAction::stop(
            invalid_tool_call_stop_message(&event.tool_name, max_attempts),
        ),
    }
}

fn invalid_tool_call_stop_message(tool: &str, max_attempts: usize) -> String {
    render_template(
        include_str!("../../../prompts/recovery/invalid_tool_call_stop.md"),
        &[
            ("{max_attempts}", max_attempts.to_string()),
            ("{tool}", tool.to_string()),
        ],
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

fn invalid_tool_call_template(force_final: Option<&str>) -> &'static str {
    match force_final {
        Some(_) => include_str!("../../../prompts/recovery/forced_invalid_tool_call_retry.md"),
        None => include_str!("../../../prompts/recovery/invalid_tool_call_retry.md"),
    }
}

fn available_tools(tools: &[String]) -> String {
    if tools.is_empty() {
        "<none>".to_string()
    } else {
        tools.join(", ")
    }
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
    let template = match force_final_reason {
        Some(_) => include_str!("../../../prompts/recovery/forced_tool_error_next_action.md"),
        None => include_str!("../../../prompts/recovery/tool_error_next_action.md"),
    };
    render_template(
        template,
        &[
            (
                "{reason}",
                force_final_reason.unwrap_or_default().to_string(),
            ),
            ("{tool}", tool_name.to_string()),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::{OneOrMany, message::ToolResultContent};

    #[test]
    fn invalid_tool_call_stop_message_has_no_escape_backslashes() {
        assert_eq!(
            invalid_tool_call_stop_message("read_file", 2),
            "model emitted an invalid tool call more than 2 times; last tool name was \"read_file\""
        );
    }

    #[test]
    fn status_marker_is_detected_without_io() {
        let result = MessageToolResult {
            id: "call-1".to_string(),
            call_id: None,
            content: OneOrMany::one(ToolResultContent::json(
                serde_json::json!({ TOOL_STATUS_FIELD: TOOL_STATUS_ERROR }),
            )),
        };
        assert_eq!(tool_recovery_status(&result), ToolRecoveryStatus::Error);
    }
}
