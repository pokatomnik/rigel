use std::{path::PathBuf, sync::Arc};

use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::{Deserialize, Serialize};

use crate::{
    entities::tool::tool_entities::{
        CommandExecutionRequest, CommandExecutionResult, PermissionRequirement,
    },
    infrastructure::tools::{
        command::{CommandExecutor, CommandUsageRecorder},
        permissions::ToolPermissionMetadata,
    },
    tools::action::Action,
};

const MAX_OUTPUT_BYTES: usize = 32 * 1024;
const OUTPUT_TRUNCATION_SUFFIX: &str = "\n[output truncated]";
const INTERNAL_FAILURE_EXIT_CODE: i32 = -1;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RunCommandArgs {
    command: String,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct RunCommandOutput {
    action: Action,
    ok: bool,
    exit_code: i32,
    output: String,
    timed_out: bool,
    truncated: bool,
}

impl RunCommandOutput {
    fn command_succeeded(exit_code: i32, timed_out: bool) -> bool {
        exit_code == 0 && !timed_out
    }

    fn from_result(result: CommandExecutionResult) -> Self {
        let (output, truncated) = Self::bounded_output(result.output, result.truncated);
        Self {
            action: Action::Completed,
            ok: Self::command_succeeded(result.exit_code, result.timed_out),
            exit_code: result.exit_code,
            output,
            timed_out: result.timed_out,
            truncated,
        }
    }

    fn bounded_output(mut output: String, was_truncated: bool) -> (String, bool) {
        let truncated = was_truncated || output.len() > MAX_OUTPUT_BYTES;
        if !truncated {
            return (output, false);
        }
        if output.ends_with(OUTPUT_TRUNCATION_SUFFIX) {
            output.truncate(output.len() - OUTPUT_TRUNCATION_SUFFIX.len());
        }

        let content_limit = MAX_OUTPUT_BYTES - OUTPUT_TRUNCATION_SUFFIX.len();
        let end = output
            .char_indices()
            .take_while(|(index, character)| *index + character.len_utf8() <= content_limit)
            .last()
            .map_or(0, |(index, character)| index + character.len_utf8());
        output.truncate(end);
        output.push_str(OUTPUT_TRUNCATION_SUFFIX);
        (output, true)
    }
}

pub(crate) struct RunCommand {
    root: PathBuf,
    executor: Arc<dyn CommandExecutor>,
    usage_recorder: Arc<dyn CommandUsageRecorder>,
    permission: PermissionRequirement,
}

impl RunCommand {
    /// Creates the command tool from composition-root-selected execution adapters.
    pub(crate) fn with_adapters(
        root: PathBuf,
        executor: Arc<dyn CommandExecutor>,
        usage_recorder: Arc<dyn CommandUsageRecorder>,
    ) -> Self {
        Self {
            root,
            executor,
            usage_recorder,
            permission: PermissionRequirement::ConfirmationRequired,
        }
    }

    async fn execute_command(&self, command: &str) -> RunCommandOutput {
        let request = CommandExecutionRequest {
            command: command.to_string(),
            current_dir: self.root.clone(),
        };
        match self.executor.execute(request).await {
            Ok(result) => RunCommandOutput::from_result(result),
            Err(error) => RunCommandOutput::from_result(CommandExecutionResult {
                exit_code: INTERNAL_FAILURE_EXIT_CODE,
                output: error.to_string(),
                timed_out: false,
                truncated: false,
            }),
        }
    }
}

impl ToolPermissionMetadata for RunCommand {
    fn permission_requirement(&self) -> PermissionRequirement {
        self.permission
    }
}

impl Tool for RunCommand {
    const NAME: &'static str = "run_command";
    type Args = RunCommandArgs;
    type Output = RunCommandOutput;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Run a shell command from the startup directory for builds, tests, git, programs, or operations not covered by a dedicated tool. For ordinary workspace file reads, searches, creation, and targeted edits, use read_file, search_files, write_file, and edit_file instead.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "command": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Shell command for builds, tests, git, programs, or operations not covered by a dedicated tool."
                }
            },
            "required": ["command"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        self.usage_recorder.record(&args.command);
        Ok(self.execute_command(&args.command).await)
    }
}

#[cfg(test)]
mod tests {
    use std::{future::Future, pin::Pin};

    use super::*;
    use crate::{
        controllers::terminal::command_executor::{CommandUsageTelemetry, ProcessCommandExecutor},
        infrastructure::tools::command::{CommandExecutor, CommandUsageRecorder},
    };

    enum StubOutcome {
        Result(CommandExecutionResult),
        Error(String),
    }

    struct StubExecutor {
        outcome: StubOutcome,
    }

    impl CommandExecutor for StubExecutor {
        fn execute(
            &self,
            _request: CommandExecutionRequest,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<CommandExecutionResult>> + Send + '_>>
        {
            match &self.outcome {
                StubOutcome::Result(result) => {
                    let result = result.clone();
                    Box::pin(async move { Ok(result) })
                }
                StubOutcome::Error(message) => {
                    let message = message.clone();
                    Box::pin(async move { Err(anyhow::anyhow!(message)) })
                }
            }
        }
    }

    struct NoopRecorder;

    impl CommandUsageRecorder for NoopRecorder {
        fn record(&self, _operation: &str) {}
    }

    fn tool(result: CommandExecutionResult) -> RunCommand {
        tool_with_outcome(StubOutcome::Result(result))
    }

    fn tool_with_outcome(outcome: StubOutcome) -> RunCommand {
        RunCommand::with_adapters(
            PathBuf::from("."),
            Arc::new(StubExecutor { outcome }),
            Arc::new(NoopRecorder),
        )
    }

    #[test]
    fn schema_contains_only_command() {
        let tool = RunCommand::with_adapters(
            PathBuf::from("."),
            Arc::new(ProcessCommandExecutor::new_default()),
            Arc::new(CommandUsageTelemetry::new()),
        );
        let schema = tool.parameters();
        assert_eq!(schema["required"], serde_json::json!(["command"]));
        assert_eq!(schema["additionalProperties"], serde_json::json!(false));
        assert_eq!(schema["properties"]["command"]["minLength"], 1);
        assert!(schema["properties"].get("timeout_ms").is_none());
    }

    #[tokio::test]
    async fn production_mapping_preserves_zero_exit_code() {
        let output = tool(CommandExecutionResult {
            exit_code: 0,
            output: "passed".to_string(),
            timed_out: false,
            truncated: false,
        })
        .execute_command("true")
        .await;

        assert!(output.ok);
        assert_eq!(output.exit_code, 0);
        assert_eq!(output.output, "passed");
    }

    #[tokio::test]
    async fn production_mapping_preserves_non_zero_exit_code() {
        let output = tool(CommandExecutionResult {
            exit_code: 2,
            output: "failed".to_string(),
            timed_out: false,
            truncated: false,
        })
        .execute_command("false")
        .await;

        assert!(!output.ok);
        assert_eq!(output.exit_code, 2);
        assert_eq!(output.action, Action::Completed);
    }

    #[tokio::test]
    async fn production_mapping_preserves_internal_failure() {
        let output = tool(CommandExecutionResult {
            exit_code: INTERNAL_FAILURE_EXIT_CODE,
            output: "internal process failure".to_string(),
            timed_out: false,
            truncated: false,
        })
        .execute_command("unavailable")
        .await;

        assert!(!output.ok);
        assert_eq!(output.exit_code, INTERNAL_FAILURE_EXIT_CODE);
        assert!(output.output.contains("internal process failure"));
    }

    #[tokio::test]
    async fn production_mapping_preserves_timeout_state() {
        let output = tool(CommandExecutionResult {
            exit_code: INTERNAL_FAILURE_EXIT_CODE,
            output: "Command timed out".to_string(),
            timed_out: true,
            truncated: false,
        })
        .execute_command("sleep")
        .await;

        assert!(!output.ok);
        assert!(output.timed_out);
        assert_eq!(output.exit_code, INTERNAL_FAILURE_EXIT_CODE);
    }

    #[tokio::test]
    async fn production_mapping_preserves_truncation_state_and_suffix() {
        let output = tool(CommandExecutionResult {
            exit_code: 0,
            output: "bounded".to_string(),
            timed_out: false,
            truncated: true,
        })
        .execute_command("large")
        .await;

        assert!(output.truncated);
        assert!(output.output.ends_with(OUTPUT_TRUNCATION_SUFFIX));
        assert!(output.output.starts_with("bounded"));
    }

    #[tokio::test]
    async fn model_facing_output_cap_includes_suffix_for_oversized_ascii() {
        let output = tool(CommandExecutionResult {
            exit_code: 0,
            output: "x".repeat(MAX_OUTPUT_BYTES + 100),
            timed_out: false,
            truncated: false,
        })
        .execute_command("large")
        .await;

        assert!(output.truncated);
        assert!(output.output.len() <= MAX_OUTPUT_BYTES);
        assert!(output.output.ends_with(OUTPUT_TRUNCATION_SUFFIX));
        assert_eq!(output.output.matches(OUTPUT_TRUNCATION_SUFFIX).count(), 1);
    }

    #[tokio::test]
    async fn model_facing_output_cap_keeps_unicode_boundary_and_suffix_once() {
        let output = tool(CommandExecutionResult {
            exit_code: 0,
            output: format!(
                "{}{}",
                "я".repeat(MAX_OUTPUT_BYTES),
                OUTPUT_TRUNCATION_SUFFIX
            ),
            timed_out: false,
            truncated: true,
        })
        .execute_command("large")
        .await;

        assert!(output.truncated);
        assert!(output.output.len() <= MAX_OUTPUT_BYTES);
        assert!(output.output.is_char_boundary(output.output.len()));
        assert!(output.output.ends_with(OUTPUT_TRUNCATION_SUFFIX));
        assert_eq!(output.output.matches(OUTPUT_TRUNCATION_SUFFIX).count(), 1);
    }

    #[tokio::test]
    async fn exact_output_boundary_is_not_marked_truncated() {
        let output = tool(CommandExecutionResult {
            exit_code: 0,
            output: "x".repeat(MAX_OUTPUT_BYTES),
            timed_out: false,
            truncated: false,
        })
        .execute_command("boundary")
        .await;

        assert!(!output.truncated);
        assert_eq!(output.output.len(), MAX_OUTPUT_BYTES);
        assert!(!output.output.ends_with(OUTPUT_TRUNCATION_SUFFIX));
    }

    #[tokio::test]
    async fn oversized_executor_error_is_bounded_with_one_suffix() {
        let diagnostic = "diagnostic ".repeat(MAX_OUTPUT_BYTES);
        let output = tool_with_outcome(StubOutcome::Error(diagnostic))
            .execute_command("failure")
            .await;

        assert_eq!(output.exit_code, INTERNAL_FAILURE_EXIT_CODE);
        assert!(output.output.starts_with("diagnostic "));
        assert!(output.truncated);
        assert!(output.output.len() <= MAX_OUTPUT_BYTES);
        assert_eq!(output.output.matches(OUTPUT_TRUNCATION_SUFFIX).count(), 1);
    }

    #[tokio::test]
    async fn thirty_second_timeout_preserves_exact_result_fields_and_text() {
        let message = "Command timed out after 30 seconds. Retry with a narrower command or check for a long-running process.";
        let output = tool(CommandExecutionResult {
            exit_code: INTERNAL_FAILURE_EXIT_CODE,
            output: message.to_string(),
            timed_out: true,
            truncated: false,
        })
        .execute_command("timeout")
        .await;

        assert_eq!(output.action, Action::Completed);
        assert!(!output.ok);
        assert_eq!(output.exit_code, INTERNAL_FAILURE_EXIT_CODE);
        assert_eq!(output.output, message);
        assert!(output.timed_out);
        assert!(!output.truncated);
    }

    #[test]
    fn telemetry_is_instance_owned_and_starts_empty() {
        let telemetry = CommandUsageTelemetry::new();
        assert_eq!(telemetry.total_calls(), 0);
    }

    #[test]
    fn ok_is_true_only_for_zero_exit_without_timeout() {
        assert!(RunCommandOutput::command_succeeded(0, false));
        assert!(!RunCommandOutput::command_succeeded(7, false));
        assert!(!RunCommandOutput::command_succeeded(0, true));
        assert!(!RunCommandOutput::command_succeeded(
            INTERNAL_FAILURE_EXIT_CODE,
            false
        ));
    }

    #[tokio::test]
    async fn timeout_and_non_zero_exit_are_result_states() {
        let timeout = tool(CommandExecutionResult {
            exit_code: INTERNAL_FAILURE_EXIT_CODE,
            output: "Command timed out".to_string(),
            timed_out: true,
            truncated: false,
        })
        .execute_command("sleep")
        .await;
        let result = tool(CommandExecutionResult {
            exit_code: 2,
            output: "failed".to_string(),
            timed_out: false,
            truncated: false,
        })
        .execute_command("false")
        .await;
        let success = tool(CommandExecutionResult {
            exit_code: 0,
            output: "passed".to_string(),
            timed_out: false,
            truncated: false,
        })
        .execute_command("true")
        .await;

        assert!(!timeout.ok);
        assert!(timeout.timed_out);
        assert!(!result.ok);
        assert_eq!(result.exit_code, 2);
        assert!(success.ok);
    }
}
