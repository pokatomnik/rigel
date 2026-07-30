use std::{
    env,
    path::{Component, Path, PathBuf},
};

use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::{Deserialize, Serialize};
use tokio::fs;

#[derive(Deserialize)]
pub(crate) struct StatArgs {
    path: String,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct StatOutput {
    path: String,
    #[serde(rename = "type")]
    kind: StatEntryType,
    size_bytes: Option<u64>,
    is_empty: Option<bool>,
    permissions: AccessPermissions,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum StatEntryType {
    File,
    Directory,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct AccessPermissions {
    readonly: bool,
    unix_mode_octal: Option<String>,
    unix_mode_symbolic: Option<String>,
}

pub(crate) struct Stat {
    root: PathBuf,
}

impl Stat {
    pub(crate) async fn new() -> Result<Self, ToolExecutionError> {
        let current_dir = env::current_dir().map_err(|error| {
            ToolExecutionError::other(format!("Cannot determine the project root: {error}"))
                .with_source(error)
        })?;
        let root = fs::canonicalize(&current_dir).await.map_err(|error| {
            ToolExecutionError::other(format!(
                "Cannot access the project root \"{}\": {error}",
                current_dir.display()
            ))
            .with_source(error)
        })?;

        Ok(Self { root })
    }

    fn normalize_relative_path(path: &str) -> Result<PathBuf, ToolExecutionError> {
        if path.is_empty() {
            return Err(ToolExecutionError::invalid_args(
                "Cannot get path information: path must not be empty. Use \".\" for the project root.",
            ));
        }

        let mut normalized = PathBuf::new();

        for component in Path::new(path).components() {
            match component {
                Component::Normal(component) => normalized.push(component),
                Component::ParentDir if !normalized.pop() => {
                    return Err(outside_project_error(path));
                }
                Component::ParentDir | Component::CurDir => {}
                Component::RootDir | Component::Prefix(_) => {
                    return Err(ToolExecutionError::invalid_args(format!(
                        "Cannot get information for \"{path}\": path must be relative to the project root."
                    )));
                }
            }
        }

        Ok(normalized)
    }

    async fn resolve_path(&self, path: &str) -> Result<(PathBuf, String), ToolExecutionError> {
        let relative_path = Self::normalize_relative_path(path)?;
        let resolved = fs::canonicalize(self.root.join(&relative_path))
            .await
            .map_err(|error| stat_error(path, error))?;

        if !resolved.starts_with(&self.root) {
            return Err(outside_project_error(path));
        }

        let relative_path = if relative_path.as_os_str().is_empty() {
            ".".to_string()
        } else {
            relative_path.to_string_lossy().into_owned()
        };

        Ok((resolved, relative_path))
    }
}

impl Tool for Stat {
    const NAME: &'static str = "stat";
    type Args = StatArgs;
    type Output = StatOutput;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Get information about a project file or directory.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "minLength": 1,
                    "description": "File or directory path relative to the project root. Use \".\" for the project root."
                }
            },
            "required": ["path"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let (resolved, path) = self.resolve_path(&args.path).await?;
        let metadata = fs::metadata(&resolved)
            .await
            .map_err(|error| stat_error(&args.path, error))?;
        let permissions = access_permissions(&metadata);

        if metadata.is_file() {
            Ok(file_result(path, metadata.len(), permissions))
        } else if metadata.is_dir() {
            let mut entries = fs::read_dir(&resolved)
                .await
                .map_err(|error| stat_error(&args.path, error))?;
            let is_empty = entries
                .next_entry()
                .await
                .map_err(|error| stat_error(&args.path, error))?
                .is_none();

            Ok(directory_result(path, is_empty, permissions))
        } else {
            Err(ToolExecutionError::invalid_args(format!(
                "Cannot get information for \"{}\": path is neither a regular file nor a directory.",
                args.path
            )))
        }
    }
}

fn file_result(path: String, size_bytes: u64, permissions: AccessPermissions) -> StatOutput {
    StatOutput {
        path,
        kind: StatEntryType::File,
        size_bytes: Some(size_bytes),
        is_empty: None,
        permissions,
    }
}

fn directory_result(path: String, is_empty: bool, permissions: AccessPermissions) -> StatOutput {
    StatOutput {
        path,
        kind: StatEntryType::Directory,
        size_bytes: None,
        is_empty: Some(is_empty),
        permissions,
    }
}

fn access_permissions(metadata: &std::fs::Metadata) -> AccessPermissions {
    let readonly = metadata.permissions().readonly();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mode = metadata.permissions().mode() & 0o7777;

        AccessPermissions {
            readonly,
            unix_mode_octal: Some(format!("{mode:04o}")),
            unix_mode_symbolic: Some(format_unix_permissions(mode)),
        }
    }

    #[cfg(not(unix))]
    {
        AccessPermissions {
            readonly,
            unix_mode_octal: None,
            unix_mode_symbolic: None,
        }
    }
}

fn format_unix_permissions(mode: u32) -> String {
    const PERMISSIONS: [(u32, char); 9] = [
        (0o400, 'r'),
        (0o200, 'w'),
        (0o100, 'x'),
        (0o040, 'r'),
        (0o020, 'w'),
        (0o010, 'x'),
        (0o004, 'r'),
        (0o002, 'w'),
        (0o001, 'x'),
    ];

    PERMISSIONS
        .iter()
        .map(|(flag, character)| {
            if mode & flag == *flag {
                *character
            } else {
                '-'
            }
        })
        .collect()
}

fn outside_project_error(path: &str) -> ToolExecutionError {
    ToolExecutionError::refused(format!(
        "Cannot get information for \"{path}\": path resolves outside the project root."
    ))
}

fn stat_error(path: &str, error: std::io::Error) -> ToolExecutionError {
    let message = match error.kind() {
        std::io::ErrorKind::NotFound => {
            format!("Cannot get information for \"{path}\": file or directory does not exist.")
        }
        std::io::ErrorKind::PermissionDenied => {
            format!("Cannot get information for \"{path}\": permission denied.")
        }
        std::io::ErrorKind::NotADirectory => {
            format!("Cannot get information for \"{path}\": a path component is not a directory.")
        }
        _ => format!("Cannot get information for \"{path}\": {error}"),
    };

    match error.kind() {
        std::io::ErrorKind::NotFound => ToolExecutionError::not_found(message),
        std::io::ErrorKind::PermissionDenied => ToolExecutionError::permission_denied(message),
        std::io::ErrorKind::NotADirectory => ToolExecutionError::invalid_args(message),
        _ => ToolExecutionError::other(message),
    }
    .with_source(error)
}

#[cfg(test)]
mod tests {
    use rig::tool::ToolErrorKind;

    use super::*;

    fn permissions() -> AccessPermissions {
        AccessPermissions {
            readonly: false,
            unix_mode_octal: Some("0755".to_string()),
            unix_mode_symbolic: Some("rwxr-xr-x".to_string()),
        }
    }

    #[test]
    fn file_result_contains_size_but_not_directory_emptiness() {
        let result = file_result("src/main.rs".to_string(), 42, permissions());

        assert_eq!(result.kind, StatEntryType::File);
        assert_eq!(result.size_bytes, Some(42));
        assert_eq!(result.is_empty, None);
    }

    #[test]
    fn directory_result_contains_emptiness_but_not_size() {
        let result = directory_result("src".to_string(), false, permissions());

        assert_eq!(result.kind, StatEntryType::Directory);
        assert_eq!(result.size_bytes, None);
        assert_eq!(result.is_empty, Some(false));
    }

    #[test]
    fn unix_permissions_are_formatted_symbolically() {
        assert_eq!(format_unix_permissions(0o755), "rwxr-xr-x");
        assert_eq!(format_unix_permissions(0o640), "rw-r-----");
        assert_eq!(format_unix_permissions(0o000), "---------");
    }

    #[test]
    fn outside_path_is_refused() {
        let error =
            Stat::normalize_relative_path("../outside").expect_err("outside path should fail");

        assert!(error.is_refusal());
        assert_eq!(
            error.model_feedback(),
            Some(
                "Cannot get information for \"../outside\": path resolves outside the project root."
            )
        );
    }

    #[test]
    fn missing_path_error_is_clear_and_model_visible() {
        let error = stat_error("missing", std::io::ErrorKind::NotFound.into());

        assert_eq!(error.kind(), ToolErrorKind::NotFound);
        assert_eq!(
            error.model_feedback(),
            Some("Cannot get information for \"missing\": file or directory does not exist.")
        );
    }
}
