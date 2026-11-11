use std::{
    future::Future,
    io::{PipeReader, PipeWriter, Read, pipe},
    pin::Pin,
    process::Stdio,
    time::Duration,
};

use tokio::{
    process::{Child, Command},
    task::{self, JoinHandle},
    time,
};

use super::shell::{UserShell, configure_command, detect_user_shell};

use crate::{
    controllers::terminal::telemetry::ShellUsageTelemetry,
    entities::tool::tool_entities::{CommandExecutionRequest, CommandExecutionResult},
    infrastructure::tools::command::{CommandExecutor, CommandUsageRecorder},
};

const MAX_OUTPUT_BYTES: usize = 32 * 1024;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const INTERNAL_FAILURE_EXIT_CODE: i32 = -1;

struct ProcessResult {
    output: String,
    exit_code: i32,
    truncated: bool,
}

/// Executes bounded shell requests using the startup environment's selected shell.
pub(crate) struct ProcessCommandExecutor {
    shell: UserShell,
    timeout: Duration,
}

impl ProcessCommandExecutor {
    /// Creates an executor using `SHELL`/`COMSPEC` with platform-safe fallbacks.
    pub(crate) fn new_default() -> Self {
        Self {
            shell: detect_user_shell(),
            timeout: COMMAND_TIMEOUT,
        }
    }

    async fn run_process(&self, request: CommandExecutionRequest) -> ProcessResult {
        let (reader, writer) = match pipe() {
            Ok(pipe) => pipe,
            Err(error) => {
                return internal_failure(format!("could not create output pipe: {error}"));
            }
        };
        let stderr_writer = match writer.try_clone() {
            Ok(writer) => writer,
            Err(error) => {
                return internal_failure(format!("could not prepare stderr pipe: {error}"));
            }
        };
        let mut child = match self.spawn(&request, writer, stderr_writer).await {
            Ok(child) => child,
            Err(error) => return internal_failure(format!("could not start shell: {error}")),
        };
        let output_task = read_output(reader);
        let status = child.wait().await;
        let (bytes, read_error, truncated) = collect_output(output_task).await;
        ProcessResult {
            output: format_output(bytes, read_error, &status),
            exit_code: exit_code(status),
            truncated,
        }
    }

    async fn spawn(
        &self,
        request: &CommandExecutionRequest,
        writer: PipeWriter,
        stderr_writer: PipeWriter,
    ) -> std::io::Result<Child> {
        let mut command = Command::new(&self.shell.executable);
        configure_command(&mut command, self.shell.kind, &request.command);
        command
            .current_dir(&request.current_dir)
            .stdin(Stdio::null())
            .stdout(writer)
            .stderr(stderr_writer)
            .kill_on_drop(true);
        command.spawn()
    }
}

impl CommandExecutor for ProcessCommandExecutor {
    fn execute(
        &self,
        request: CommandExecutionRequest,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<CommandExecutionResult>> + Send + '_>> {
        Box::pin(
            async move { Ok(execute_with_timeout(self.run_process(request), self.timeout).await) },
        )
    }
}

/// Records shell usage through one composition-root-owned telemetry instance.
pub(crate) struct CommandUsageTelemetry {
    telemetry: ShellUsageTelemetry,
}

impl CommandUsageTelemetry {
    /// Creates an isolated recorder whose counters start at zero.
    pub(crate) fn new() -> Self {
        Self {
            telemetry: ShellUsageTelemetry::new(),
        }
    }

    /// Returns the counters accumulated by this recorder only.
    #[cfg(test)]
    pub(crate) fn total_calls(&self) -> usize {
        self.telemetry.total_calls()
    }
}

async fn execute_with_timeout<F>(process: F, timeout: Duration) -> CommandExecutionResult
where
    F: Future<Output = ProcessResult>,
{
    match time::timeout(timeout, process).await {
        Ok(result) => completed_result(result),
        Err(_) => timed_out_result(timeout),
    }
}

fn completed_result(result: ProcessResult) -> CommandExecutionResult {
    let (output, truncated) = limit_output(result.output, result.truncated);
    CommandExecutionResult {
        exit_code: result.exit_code,
        output,
        timed_out: false,
        truncated,
    }
}

fn timed_out_result(timeout: Duration) -> CommandExecutionResult {
    CommandExecutionResult {
        exit_code: INTERNAL_FAILURE_EXIT_CODE,
        output: format!(
            "Command timed out after {} seconds. Retry with a narrower command or check for a long-running process.",
            timeout.as_secs()
        ),
        timed_out: true,
        truncated: false,
    }
}

impl CommandUsageRecorder for CommandUsageTelemetry {
    fn record(&self, operation: &str) {
        self.telemetry.record_command(operation);
    }
}

fn read_output(mut reader: PipeReader) -> JoinHandle<(Vec<u8>, std::io::Result<()>, bool)> {
    task::spawn_blocking(move || {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 8 * 1024];
        let mut truncated = false;
        let result = loop {
            match reader.read(&mut buffer) {
                Ok(0) => break Ok(()),
                Ok(count) => {
                    let remaining = MAX_OUTPUT_BYTES.saturating_sub(bytes.len());
                    let kept = count.min(remaining);
                    bytes.extend_from_slice(&buffer[..kept]);
                    truncated |= count > kept;
                }
                Err(error) => break Err(error),
            }
        };
        (bytes, result, truncated)
    })
}

async fn collect_output(
    output_task: JoinHandle<(Vec<u8>, std::io::Result<()>, bool)>,
) -> (Vec<u8>, Option<String>, bool) {
    match output_task.await {
        Ok((bytes, Ok(()), truncated)) => (bytes, None, truncated),
        Ok((bytes, Err(error), truncated)) => (
            bytes,
            Some(format!("could not read command output: {error}")),
            truncated,
        ),
        Err(error) => (
            Vec::new(),
            Some(format!("could not collect command output: {error}")),
            false,
        ),
    }
}

fn format_output(
    bytes: Vec<u8>,
    read_error: Option<String>,
    status: &std::io::Result<std::process::ExitStatus>,
) -> String {
    let mut output = String::from_utf8_lossy(&bytes).into_owned();
    if let Some(error) = read_error {
        append_diagnostic(&mut output, &error);
    }
    if let Err(error) = status {
        append_diagnostic(
            &mut output,
            &format!("could not obtain command exit status: {error}"),
        );
    }
    output
}

fn exit_code(status: std::io::Result<std::process::ExitStatus>) -> i32 {
    match status {
        Ok(status) => status.code().unwrap_or(INTERNAL_FAILURE_EXIT_CODE),
        Err(_) => INTERNAL_FAILURE_EXIT_CODE,
    }
}

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

fn append_diagnostic(output: &mut String, message: &str) {
    if !output.is_empty() && !output.ends_with('\n') {
        output.push('\n');
    }
    output.push_str("run_command: ");
    output.push_str(message);
    output.push('\n');
}

fn limit_output(output: String, already_truncated: bool) -> (String, bool) {
    if !already_truncated && output.len() <= MAX_OUTPUT_BYTES {
        return (output, false);
    }
    let end = output
        .char_indices()
        .take_while(|(index, _)| *index < MAX_OUTPUT_BYTES)
        .last()
        .map_or(0, |(index, character)| index + character.len_utf8());
    (output[..end].to_string(), true)
}

#[cfg(test)]
mod tests {
    use std::{future, time::Duration};

    use super::{
        INTERNAL_FAILURE_EXIT_CODE, MAX_OUTPUT_BYTES, ProcessResult, execute_with_timeout,
        limit_output,
    };

    #[test]
    fn output_limit_is_explicit_and_utf8_safe() {
        let (output, truncated) = limit_output("я".repeat(MAX_OUTPUT_BYTES), false);
        assert!(truncated);
        assert!(output.is_char_boundary(output.len()));
        assert!(output.len() <= MAX_OUTPUT_BYTES);
    }

    #[tokio::test]
    async fn executor_preserves_non_zero_exit_code() {
        let result = execute_with_timeout(
            future::ready(ProcessResult {
                output: "failed".to_string(),
                exit_code: 2,
                truncated: false,
            }),
            Duration::from_secs(30),
        )
        .await;
        assert_eq!(result.exit_code, 2);
        assert!(!result.timed_out);
    }

    #[tokio::test]
    async fn executor_reports_timeout_and_truncation() {
        let timeout = execute_with_timeout(future::pending(), Duration::from_millis(10)).await;
        assert!(timeout.timed_out);
        assert_eq!(timeout.exit_code, INTERNAL_FAILURE_EXIT_CODE);

        let bounded = execute_with_timeout(
            future::ready(ProcessResult {
                output: "bounded".repeat(MAX_OUTPUT_BYTES),
                exit_code: 0,
                truncated: false,
            }),
            Duration::from_secs(30),
        )
        .await;
        assert!(bounded.truncated);
        assert!(bounded.output.len() <= MAX_OUTPUT_BYTES + 1);
    }
}
