use std::{
    env,
    ffi::{OsStr, OsString},
    io::{PipeReader, PipeWriter, Read, pipe},
    path::PathBuf,
    process::Stdio,
    time::Duration,
};

use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::{Deserialize, Serialize};
use tokio::{
    fs,
    process::{Child, Command},
    task::{self, JoinHandle},
};

use crate::shared::tool_permissions::{PermissionRequirement, ToolPermissionMetadata};

use super::contracts::{Action, error_codes};

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
    exit_code: i32,
    output: String,
    timed_out: bool,
    truncated: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShellKind {
    Posix,
    PowerShell,
    Cmd,
}

#[derive(Debug, PartialEq, Eq)]
struct UserShell {
    executable: OsString,
    name: String,
    kind: ShellKind,
}

struct ProcessResult {
    output: String,
    exit_code: i32,
    truncated: bool,
}

pub(crate) struct RunCommand {
    root: PathBuf,
    shell: UserShell,
    permission: PermissionRequirement,
}

impl RunCommand {
    pub(crate) async fn new() -> Result<Self, ToolExecutionError> {
        let current_dir = env::current_dir().map_err(|error| {
            ToolExecutionError::other(format!("Cannot determine the current directory: {error}"))
                .with_code(error_codes::IO_ERROR)
                .with_source(error)
        })?;
        let root = fs::canonicalize(&current_dir).await.map_err(|error| {
            ToolExecutionError::other(format!(
                "Cannot access the current directory \"{}\": {error}",
                current_dir.display()
            ))
            .with_code(error_codes::IO_ERROR)
            .with_source(error)
        })?;
        Ok(Self {
            root,
            shell: detect_user_shell(),
            permission: PermissionRequirement::ConfirmationRequired,
        })
    }

    async fn execute_with_timeout(&self, command: &str) -> RunCommandOutput {
        match tokio::time::timeout(SERVER_TIMEOUT, self.execute(command)).await {
            Ok(result) => completed_result(result),
            Err(_) => timeout_result(),
        }
    }

    async fn execute(&self, command: &str) -> ProcessResult {
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
        let mut child = match self.spawn(command, writer, stderr_writer).await {
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
        command_text: &str,
        writer: PipeWriter,
        stderr_writer: PipeWriter,
    ) -> std::io::Result<Child> {
        let mut command = Command::new(&self.shell.executable);
        configure_command(&mut command, self.shell.kind, command_text);
        command
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .stdout(writer)
            .stderr(stderr_writer)
            .kill_on_drop(true);
        command.spawn()
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
        "Run a shell command in the current directory, including filesystem operations such as ls, rg, find, rm, touch, and patch.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "command": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Shell command to run for builds, tests, programs, or filesystem operations."
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
        Ok(self.execute_with_timeout(&args.command).await)
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

fn completed_result(result: ProcessResult) -> RunCommandOutput {
    let (output, truncated) = limit_output(result.output, result.truncated);
    RunCommandOutput {
        action: Action::Completed,
        exit_code: result.exit_code,
        output,
        timed_out: false,
        truncated,
    }
}

fn timeout_result() -> RunCommandOutput {
    RunCommandOutput {
        action: Action::Completed,
        exit_code: INTERNAL_FAILURE_EXIT_CODE,
        output: format!(
            "Command timed out after {} seconds.",
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

fn internal_failure(message: String) -> ProcessResult {
    let mut output = String::new();
    append_diagnostic(&mut output, &message);
    ProcessResult {
        output,
        exit_code: INTERNAL_FAILURE_EXIT_CODE,
        truncated: false,
    }
}

fn detect_user_shell() -> UserShell {
    select_user_shell(
        env::var_os("SHELL").as_deref(),
        env::var_os("COMSPEC").as_deref(),
        cfg!(windows),
    )
}

fn select_user_shell(
    shell: Option<&OsStr>,
    comspec: Option<&OsStr>,
    is_windows: bool,
) -> UserShell {
    if let Some(shell) = non_empty(shell) {
        return user_shell(shell, is_windows);
    }
    if is_windows {
        return user_shell(
            non_empty(comspec).unwrap_or_else(|| OsStr::new("cmd.exe")),
            true,
        );
    }
    user_shell(OsStr::new("/bin/sh"), false)
}

fn non_empty(value: Option<&OsStr>) -> Option<&OsStr> {
    value.filter(|value| !value.is_empty())
}

fn user_shell(executable: &OsStr, is_windows: bool) -> UserShell {
    let name = executable_name(executable).to_ascii_lowercase();
    let kind = match name.as_str() {
        "pwsh" | "powershell" => ShellKind::PowerShell,
        "cmd" => ShellKind::Cmd,
        _ if is_windows => ShellKind::Cmd,
        _ => ShellKind::Posix,
    };
    UserShell {
        executable: executable.to_owned(),
        name,
        kind,
    }
}

fn executable_name(executable: &OsStr) -> String {
    let executable = executable.to_string_lossy();
    let file_name = executable.rsplit(['/', '\\']).next().unwrap_or(&executable);
    let extension_start = file_name.len().saturating_sub(4);
    if file_name
        .get(extension_start..)
        .is_some_and(|extension| extension.eq_ignore_ascii_case(".exe"))
    {
        file_name[..extension_start].to_string()
    } else {
        file_name.to_string()
    }
}

fn configure_command(command: &mut Command, shell: ShellKind, command_text: &str) {
    match shell {
        ShellKind::Posix => command.args(["-c", command_text]),
        ShellKind::PowerShell => command.args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            command_text,
        ]),
        ShellKind::Cmd => command.args(["/D", "/S", "/C", command_text]),
    };
}

fn exit_code(status: std::io::Result<std::process::ExitStatus>) -> i32 {
    status
        .ok()
        .and_then(|status| status.code())
        .unwrap_or(INTERNAL_FAILURE_EXIT_CODE)
}

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

    #[test]
    fn schema_contains_only_command() {
        let tool = RunCommand {
            root: PathBuf::from("."),
            shell: user_shell(OsStr::new("sh"), false),
            permission: PermissionRequirement::ConfirmationRequired,
        };
        let schema = tool.parameters();
        assert_eq!(schema["required"], serde_json::json!(["command"]));
        assert_eq!(schema["additionalProperties"], serde_json::json!(false));
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

        assert!(timeout.timed_out);
        assert_eq!(result.exit_code, 2);
        assert_eq!(result.action, Action::Completed);
    }

    #[test]
    fn shell_selection_remains_platform_aware() {
        let shell = select_user_shell(
            Some(OsStr::new("/opt/homebrew/bin/zsh")),
            Some(OsStr::new("cmd.exe")),
            false,
        );

        assert_eq!(shell.name, "zsh");
        assert_eq!(shell.kind, ShellKind::Posix);
    }
}
