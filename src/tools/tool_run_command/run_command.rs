use std::{path::PathBuf, sync::Arc, time::Duration};

use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::{Deserialize, Serialize};

use crate::{
    contracts::tools::{
        command::{CommandExecutor, CommandUsageRecorder},
        permissions::ToolPermissionMetadata,
    },
    entities::tool::tool_entities::{
        CommandExecutionRequest, CommandExecutionResult, PermissionRequirement,
    },
    tools::action::Action,
};

const SERVER_TIMEOUT: Duration = Duration::from_secs(30);
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

#[cfg(test)]
struct ProcessResult {
    output: String,
    exit_code: i32,
    truncated: bool,
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

    async fn execute_with_timeout(&self, command: &str) -> RunCommandOutput {
        let request = CommandExecutionRequest {
            command: command.to_string(),
            current_dir: self.root.clone(),
        };
        match tokio::time::timeout(SERVER_TIMEOUT, self.executor.execute(request)).await {
            Ok(Ok(result)) => contract_result(result),
            Ok(Err(error)) => contract_result(CommandExecutionResult {
                success: false,
                output: error.to_string(),
                timed_out: false,
            }),
            Err(_) => timeout_result(),
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
        Ok(self.execute_with_timeout(&args.command).await)
    }
}

fn contract_result(result: CommandExecutionResult) -> RunCommandOutput {
    let exit_code = if result.success {
        0
    } else {
        INTERNAL_FAILURE_EXIT_CODE
    };
    let already_truncated = result.output.ends_with(OUTPUT_TRUNCATION_SUFFIX);
    let (output, truncated) = limit_output(result.output, already_truncated);
    RunCommandOutput {
        action: Action::Completed,
        ok: command_succeeded(exit_code, result.timed_out),
        exit_code,
        output,
        timed_out: result.timed_out,
        truncated,
    }
}

#[cfg(test)]
fn completed_result(result: ProcessResult) -> RunCommandOutput {
    let (output, truncated) = limit_output(result.output, result.truncated);
    RunCommandOutput {
        action: Action::Completed,
        ok: command_succeeded(result.exit_code, false),
        exit_code: result.exit_code,
        output,
        timed_out: false,
        truncated,
    }
}

fn timeout_result() -> RunCommandOutput {
    RunCommandOutput {
        action: Action::Completed,
        ok: command_succeeded(INTERNAL_FAILURE_EXIT_CODE, true),
        exit_code: INTERNAL_FAILURE_EXIT_CODE,
        output: format!(
            "Command timed out after {} seconds. Retry with a narrower command or check for a long-running process.",
            SERVER_TIMEOUT.as_secs()
        ),
        timed_out: true,
        truncated: false,
    }
}

fn limit_output(output: String, already_truncated: bool) -> (String, bool) {
    if !already_truncated && output.len() <= MAX_OUTPUT_BYTES {
        return (output, false);
    }
    let content_limit = MAX_OUTPUT_BYTES.saturating_sub(OUTPUT_TRUNCATION_SUFFIX.len());
    let end = output
        .char_indices()
        .take_while(|(index, _)| *index < content_limit)
        .last()
        .map_or(0, |(index, character)| index + character.len_utf8());
    (
        format!("{}{}", &output[..end], OUTPUT_TRUNCATION_SUFFIX),
        true,
    )
}

fn command_succeeded(exit_code: i32, timed_out: bool) -> bool {
    exit_code == 0 && !timed_out
}

#[cfg(test)]
fn internal_failure(message: String) -> ProcessResult {
    let mut output = String::new();
    append_diagnostic(&mut output, &format!("internal process failure: {message}"));
    append_diagnostic(
        &mut output,
        "Check the configured shell and retry the command.",
    );
    ProcessResult {
        output,
        exit_code: INTERNAL_FAILURE_EXIT_CODE,
        truncated: false,
    }
}

#[cfg(test)]
fn append_diagnostic(output: &mut String, message: &str) {
    if !output.is_empty() && !output.ends_with('\n') {
        output.push('\n');
    }
    output.push_str("run_command: ");
    output.push_str(message);
    output.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::terminal::command_executor::{
        GlobalShellUsageRecorder, ProcessCommandExecutor,
    };

    #[test]
    fn schema_contains_only_command() {
        let tool = RunCommand::with_adapters(
            PathBuf::from("."),
            Arc::new(ProcessCommandExecutor::new_default()),
            Arc::new(GlobalShellUsageRecorder::new()),
        );
        let schema = tool.parameters();
        assert_eq!(schema["required"], serde_json::json!(["command"]));
        assert_eq!(schema["additionalProperties"], serde_json::json!(false));
        assert_eq!(schema["properties"]["command"]["minLength"], 1);
        assert_eq!(
            schema["properties"]["command"]["description"],
            "Shell command for builds, tests, git, programs, or operations not covered by a dedicated tool."
        );
        assert!(schema["properties"].get("timeout_ms").is_none());
    }

    #[test]
    fn output_limit_is_explicit_and_utf8_safe() {
        let (output, truncated) = limit_output("я".repeat(MAX_OUTPUT_BYTES), false);

        assert!(truncated);
        assert!(output.ends_with("[output truncated]"));
        assert!(output.is_char_boundary(output.len()));
    }

    #[test]
    fn timeout_and_non_zero_exit_are_result_states() {
        let timeout = timeout_result();
        let result = completed_result(ProcessResult {
            output: "failed".to_string(),
            exit_code: 2,
            truncated: false,
        });
        let success = completed_result(ProcessResult {
            output: "passed".to_string(),
            exit_code: 0,
            truncated: false,
        });
        let internal = completed_result(internal_failure("shell unavailable".to_string()));

        assert_eq!(
            serde_json::to_value(success)
                .ok()
                .and_then(|value| value.get("ok").cloned()),
            Some(serde_json::json!(true))
        );
        assert!(!timeout.ok);
        assert!(timeout.timed_out);
        assert!(!result.ok);
        assert_eq!(result.exit_code, 2);
        assert_eq!(result.action, Action::Completed);
        assert!(!internal.ok);
        assert!(!internal.timed_out);
        assert_eq!(internal.exit_code, INTERNAL_FAILURE_EXIT_CODE);
        assert!(internal.output.contains("internal process failure"));
    }

    #[test]
    fn ok_is_true_only_for_zero_exit_without_timeout() {
        assert!(command_succeeded(0, false));
        assert!(!command_succeeded(7, false));
        assert!(!command_succeeded(0, true));
        assert!(!command_succeeded(INTERNAL_FAILURE_EXIT_CODE, false));
    }
}
