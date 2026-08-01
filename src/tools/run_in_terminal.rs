use std::{
    env,
    ffi::{OsStr, OsString},
    io::{Read, pipe},
    path::PathBuf,
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::{Deserialize, Serialize};
use tokio::{fs, process::Command, task};

use crate::{entities::tool_confirm_result::ToolConfirmResult, shared::terminal_io::TerminalIO};

const INTERNAL_FAILURE_EXIT_CODE: i32 = -1;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RunInTerminalArgs {
    code: String,
    timeout_milliseconds: Option<u64>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct RunInTerminalOutput {
    shell: String,
    output: String,
    exit_code: i32,
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

pub(crate) struct RunInTerminal {
    root: PathBuf,
    shell: UserShell,
    terminal_io: Arc<TerminalIO>,
}

impl RunInTerminal {
    pub(crate) async fn new(terminal_io: Arc<TerminalIO>) -> Result<Self, ToolExecutionError> {
        let current_dir = env::current_dir().map_err(|error| {
            ToolExecutionError::other(format!(
                "Cannot determine the workspace root for run_in_terminal: {error}"
            ))
            .with_source(error)
        })?;
        let root = fs::canonicalize(&current_dir).await.map_err(|error| {
            ToolExecutionError::other(format!(
                "Cannot access the workspace root \"{}\" for run_in_terminal: {error}",
                current_dir.display()
            ))
            .with_source(error)
        })?;

        Ok(Self {
            root,
            shell: detect_user_shell(),
            terminal_io,
        })
    }

    pub(crate) fn confirm(&self, command: &str) -> ToolConfirmResult {
        self.terminal_io.confirm_toll_call(
            format!("Are you sure about running this command in the terminal \"{command}\"?")
                .as_str(),
        )
    }

    async fn execute(&self, code: &str) -> RunInTerminalOutput {
        let (mut reader, writer) = match pipe() {
            Ok(pipe) => pipe,
            Err(error) => {
                return internal_failure(
                    &self.shell.name,
                    format!("could not create the output pipe: {error}"),
                );
            }
        };
        let stderr_writer = match writer.try_clone() {
            Ok(writer) => writer,
            Err(error) => {
                return internal_failure(
                    &self.shell.name,
                    format!("could not prepare the stderr pipe: {error}"),
                );
            }
        };

        let mut command = Command::new(&self.shell.executable);
        configure_command(&mut command, self.shell.kind, code);
        command
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .stdout(writer)
            .stderr(stderr_writer)
            .kill_on_drop(true);

        let child = command.spawn();
        drop(command);

        let mut child = match child {
            Ok(child) => child,
            Err(error) => {
                return internal_failure(
                    &self.shell.name,
                    format!("could not start the shell: {error}"),
                );
            }
        };

        let output_task = task::spawn_blocking(move || {
            let mut bytes = Vec::new();
            let result = reader.read_to_end(&mut bytes);
            (bytes, result)
        });

        let status = child.wait().await;
        let (bytes, read_result) = match output_task.await {
            Ok(result) => result,
            Err(error) => {
                let mut output = String::new();
                append_diagnostic(
                    &mut output,
                    &format!("could not collect shell output: {error}"),
                );
                return RunInTerminalOutput {
                    shell: self.shell.name.clone(),
                    output,
                    exit_code: exit_code(status),
                };
            }
        };

        let mut output = String::from_utf8_lossy(&bytes).into_owned();
        if let Err(error) = read_result {
            append_diagnostic(
                &mut output,
                &format!("could not read all shell output: {error}"),
            );
        }
        if let Err(error) = &status {
            append_diagnostic(
                &mut output,
                &format!("could not obtain the shell exit status: {error}"),
            );
        }

        RunInTerminalOutput {
            shell: self.shell.name.clone(),
            output,
            exit_code: exit_code(status),
        }
    }

    /// Runs the command, aborting with a timeout error when the shell does not finish in time.
    ///
    /// A timed-out run kills the spawned shell process (`kill_on_drop`), so no command keeps
    /// running in the background while the tool reports `TOOL_TIMED_OUT`.
    async fn execute_with_timeout(
        &self,
        code: &str,
        timeout_milliseconds: Option<u64>,
    ) -> Result<RunInTerminalOutput, ToolExecutionError> {
        let Some(timeout) = timeout_milliseconds else {
            return Ok(self.execute(code).await);
        };

        tokio::select! {
            biased;

            result = self.execute(code) => Ok(result),
            _ = tokio::time::sleep(Duration::from_millis(timeout)) => Err(ToolExecutionError::timeout("Tool timed out").with_code("TOOL_TIMED_OUT"))
        }
    }
}

impl Tool for RunInTerminal {
    const NAME: &'static str = "run_in_terminal";
    type Args = RunInTerminalArgs;
    type Output = RunInTerminalOutput;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Run code in the user's shell from the workspace root. Returns combined stdout and stderr in their original order and the shell exit code. A non-zero exit code is a command result, not a tool error."
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "code": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Code to execute in the user's shell."
                },
                "timeout_milliseconds": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Optional operation timeout"
                }
            },
            "required": ["code"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let confirmed = self.confirm(args.code.as_str());

        if let ToolConfirmResult::No = confirmed {
            return Err(user_forbid());
        }
        self.execute_with_timeout(args.code.as_str(), args.timeout_milliseconds)
            .await
    }
}

fn user_forbid() -> ToolExecutionError {
    ToolExecutionError::refused(format!(
        "The user has prohibited deletion of this directory. Try another method or ask the user what to do instead."
    )).with_code("USER_FORBID")
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
        let shell = non_empty(comspec).unwrap_or_else(|| OsStr::new("cmd.exe"));
        return user_shell(shell, true);
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

fn configure_command(command: &mut Command, shell: ShellKind, code: &str) {
    match shell {
        ShellKind::Posix => {
            command.args(["-c", code]);
        }
        ShellKind::PowerShell => {
            command.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", code]);
        }
        ShellKind::Cmd => {
            command.args(["/D", "/S", "/C", code]);
        }
    }
}

fn exit_code(status: std::io::Result<std::process::ExitStatus>) -> i32 {
    status
        .ok()
        .and_then(|status| status.code())
        .unwrap_or(INTERNAL_FAILURE_EXIT_CODE)
}

fn internal_failure(shell: &str, message: String) -> RunInTerminalOutput {
    let mut output = String::new();
    append_diagnostic(&mut output, &message);

    RunInTerminalOutput {
        shell: shell.to_string(),
        output,
        exit_code: INTERNAL_FAILURE_EXIT_CODE,
    }
}

fn append_diagnostic(output: &mut String, message: &str) {
    if !output.is_empty() && !output.ends_with('\n') {
        output.push('\n');
    }
    output.push_str("run_in_terminal: ");
    output.push_str(message);
    output.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_zsh_from_shell_environment() {
        let shell = select_user_shell(
            Some(OsStr::new("/opt/homebrew/bin/zsh")),
            Some(OsStr::new("C:\\Windows\\System32\\cmd.exe")),
            false,
        );

        assert_eq!(shell.executable, OsString::from("/opt/homebrew/bin/zsh"));
        assert_eq!(shell.name, "zsh");
        assert_eq!(shell.kind, ShellKind::Posix);
    }

    #[test]
    fn recognizes_windows_shell_names_independently_of_host_platform() {
        let powershell = user_shell(OsStr::new("C:\\Program Files\\PowerShell\\PwSh.ExE"), true);
        let cmd = user_shell(OsStr::new("C:\\Windows\\System32\\cmd.exe"), true);

        assert_eq!(powershell.name, "pwsh");
        assert_eq!(powershell.kind, ShellKind::PowerShell);
        assert_eq!(cmd.name, "cmd");
        assert_eq!(cmd.kind, ShellKind::Cmd);
    }

    #[test]
    fn falls_back_to_comspec_on_windows() {
        let shell = select_user_shell(
            None,
            Some(OsStr::new("C:\\Windows\\System32\\cmd.exe")),
            true,
        );

        assert_eq!(
            shell.executable,
            OsString::from("C:\\Windows\\System32\\cmd.exe")
        );
        assert_eq!(shell.name, "cmd");
        assert_eq!(shell.kind, ShellKind::Cmd);
    }

    #[test]
    fn falls_back_to_sh_on_unix() {
        let shell = select_user_shell(None, None, false);

        assert_eq!(shell.executable, OsString::from("/bin/sh"));
        assert_eq!(shell.name, "sh");
        assert_eq!(shell.kind, ShellKind::Posix);
    }

    #[test]
    fn diagnostic_starts_on_a_new_line_without_changing_existing_output() {
        let mut output = String::from("stdout\nstderr");

        append_diagnostic(&mut output, "status unavailable");

        assert_eq!(
            output,
            "stdout\nstderr\nrun_in_terminal: status unavailable\n"
        );
    }

    #[test]
    fn timeout_is_optional_in_args() {
        let args: RunInTerminalArgs = serde_json::from_value(serde_json::json!({
            "code": "echo hello"
        }))
        .expect("arguments should deserialize");

        assert_eq!(args.timeout_milliseconds, None);
    }

    #[test]
    fn timeout_is_parsed_from_args() {
        let args: RunInTerminalArgs = serde_json::from_value(serde_json::json!({
            "code": "echo hello",
            "timeout_milliseconds": 5000
        }))
        .expect("arguments should deserialize");

        assert_eq!(args.timeout_milliseconds, Some(5000));
    }
}
