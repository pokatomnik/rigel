use std::{
    future::Future,
    io::{PipeReader, PipeWriter, Read, pipe},
    pin::Pin,
    process::Stdio,
};

use tokio::{
    process::{Child, Command},
    task::{self, JoinHandle},
};

use super::shell::{UserShell, configure_command, detect_user_shell};

use crate::{
    adapters::terminal::telemetry::ShellUsageTelemetry,
    contracts::tools::command::{CommandExecutor, CommandUsageRecorder},
    entities::tool::tool_entities::{CommandExecutionRequest, CommandExecutionResult},
};

const MAX_OUTPUT_BYTES: usize = 32 * 1024;
const INTERNAL_FAILURE_EXIT_CODE: i32 = -1;

struct ProcessResult {
    output: String,
    exit_code: i32,
    truncated: bool,
}

/// Executes bounded shell requests using the startup environment's selected shell.
pub(crate) struct ProcessCommandExecutor {
    shell: UserShell,
}

impl ProcessCommandExecutor {
    /// Creates an executor using `SHELL`/`COMSPEC` with platform-safe fallbacks.
    pub(crate) fn new_default() -> Self {
        Self {
            shell: detect_user_shell(),
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
        Box::pin(async move {
            let result = self.run_process(request).await;
            let (output, _) = limit_output(result.output, result.truncated);
            Ok(CommandExecutionResult {
                success: result.exit_code == 0,
                output,
                timed_out: false,
            })
        })
    }
}

/// Records shell usage through Rigel's process-wide telemetry sink.
pub(crate) struct GlobalShellUsageRecorder;

impl GlobalShellUsageRecorder {
    /// Creates the default recorder used by the composition root.
    pub(crate) const fn new() -> Self {
        Self
    }
}

impl CommandUsageRecorder for GlobalShellUsageRecorder {
    fn record(&self, operation: &str) {
        ShellUsageTelemetry::global().record_command(operation);
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
