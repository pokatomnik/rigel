use std::ffi::{OsStr, OsString};

use tokio::process::Command;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ShellKind {
    Posix,
    PowerShell,
    Cmd,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct UserShell {
    pub(super) executable: OsString,
    pub(super) name: String,
    pub(super) kind: ShellKind,
}

pub(super) fn detect_user_shell() -> UserShell {
    select_user_shell(
        std::env::var_os("SHELL").as_deref(),
        std::env::var_os("COMSPEC").as_deref(),
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

pub(super) fn configure_command(command: &mut Command, shell: ShellKind, command_text: &str) {
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

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::{ShellKind, select_user_shell};

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
