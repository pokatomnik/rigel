use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

use futures::future::BoxFuture;
use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::{Deserialize, Serialize};

use crate::shared::tool_permissions::catalog::{PermissionRequirement, ToolPermissionMetadata};
use crate::tools::{
    action::Action,
    utils::{
        errors::{io_error, path_outside_workspace},
        filesystem::{
            DirectoryEntries, PathKind, is_excluded_directory, path_kind, read_directory,
            startup_root_with,
        },
        path::{display_relative_path, is_inside},
    },
};

use super::{pattern::GlobPattern, scan::GlobScan};

const MAX_DIRECTORY_ENTRIES: usize = 2_048;
const MAX_DIRECTORIES: usize = 10_000;

/// Arguments for discovering regular workspace files by a relative glob pattern.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GlobFilesArgs {
    pattern: String,
}

/// Stable model-facing result for a successful bounded filename discovery.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct GlobFilesOutput {
    /// Identifies this successful response as filename matching.
    pub(crate) action: Action,
    /// Indicates that validation and bounded discovery completed successfully.
    pub(crate) ok: bool,
    /// Contains the normalized relative pattern used for matching.
    pub(crate) pattern: String,
    /// Contains sorted, unique relative paths of returned regular files.
    pub(crate) files: Vec<String>,
    /// Counts only the paths present in `files`.
    pub(crate) match_count: usize,
    /// Indicates that a server scan or result limit made the list incomplete.
    pub(crate) truncated: bool,
    /// Explains the server limit when `truncated` is true.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) message: Option<String>,
}

trait FileSystem: Send + Sync {
    fn canonicalize(&self, path: PathBuf) -> BoxFuture<'static, io::Result<PathBuf>>;
    fn path_kind(&self, path: PathBuf) -> BoxFuture<'static, io::Result<PathKind>>;
    fn read_directory(&self, path: PathBuf) -> BoxFuture<'static, io::Result<DirectoryEntries>>;
}

struct TokioFileSystem;

impl FileSystem for TokioFileSystem {
    fn canonicalize(&self, path: PathBuf) -> BoxFuture<'static, io::Result<PathBuf>> {
        Box::pin(tokio::fs::canonicalize(path))
    }

    fn path_kind(&self, path: PathBuf) -> BoxFuture<'static, io::Result<PathKind>> {
        Box::pin(async move { path_kind(&path).await })
    }

    fn read_directory(&self, path: PathBuf) -> BoxFuture<'static, io::Result<DirectoryEntries>> {
        Box::pin(async move { read_directory(&path, MAX_DIRECTORY_ENTRIES).await })
    }
}

/// Read-only, bounded filename discovery rooted at Rigel's startup workspace.
pub(crate) struct GlobFiles {
    root: PathBuf,
    file_system: Arc<dyn FileSystem>,
    permission: PermissionRequirement,
}

impl GlobFiles {
    /// Creates an automatic-permission tool rooted at the canonical startup directory.
    pub(crate) async fn new() -> Result<Self, ToolExecutionError> {
        let file_system: Arc<dyn FileSystem> = Arc::new(TokioFileSystem);
        let root = startup_root_with(|path| file_system.canonicalize(path))
            .await
            .map_err(|error| io_error("workspace", error, "glob"))?;
        Ok(Self {
            root,
            file_system,
            permission: PermissionRequirement::Automatic,
        })
    }

    async fn glob_path(
        &self,
        requested_pattern: &str,
    ) -> Result<GlobFilesOutput, ToolExecutionError> {
        let pattern = GlobPattern::parse(requested_pattern)?;
        let mut scan = GlobScan::new(pattern);
        scan.add_root(self.root.clone());
        self.walk(&mut scan).await?;
        Ok(scan.output())
    }

    async fn walk(&self, scan: &mut GlobScan) -> Result<(), ToolExecutionError> {
        while let Some(directory) = scan.next_directory() {
            if scan.directories_reached(MAX_DIRECTORIES) {
                scan.mark_truncated();
                break;
            }
            scan.count_directory();
            self.scan_directory(directory, scan).await?;
            if scan.should_stop() {
                break;
            }
        }
        Ok(())
    }

    async fn scan_directory(
        &self,
        directory: PathBuf,
        scan: &mut GlobScan,
    ) -> Result<(), ToolExecutionError> {
        let entries = self.read_directory(directory.clone()).await?;
        let was_truncated = entries.truncated;
        let mut paths = entries.paths;
        paths.sort_by(|left, right| left.to_string_lossy().cmp(&right.to_string_lossy()));
        for path in paths {
            if scan.should_stop() {
                break;
            }
            self.scan_entry(path, scan).await?;
        }
        if was_truncated {
            scan.mark_truncated();
            scan.stop();
        }
        scan.sort_pending();
        Ok(())
    }

    async fn scan_entry(
        &self,
        path: PathBuf,
        scan: &mut GlobScan,
    ) -> Result<(), ToolExecutionError> {
        let kind = self.path_kind(path.clone()).await?;
        match kind {
            PathKind::File => self.record_file(path, scan).await,
            PathKind::Directory if is_excluded_directory(&path) => Ok(()),
            PathKind::Directory => self.queue_directory(path, scan).await,
            PathKind::Symlink | PathKind::Other => Ok(()),
        }
    }

    async fn queue_directory(
        &self,
        path: PathBuf,
        scan: &mut GlobScan,
    ) -> Result<(), ToolExecutionError> {
        let canonical = self.canonical_entry(&path).await?;
        scan.queue_directory(canonical);
        Ok(())
    }

    async fn record_file(
        &self,
        path: PathBuf,
        scan: &mut GlobScan,
    ) -> Result<(), ToolExecutionError> {
        let canonical = self.canonical_entry(&path).await?;
        let relative = canonical
            .strip_prefix(&self.root)
            .map_err(|_| path_outside_workspace(&path.to_string_lossy(), "glob"))?;
        let display = display_relative_path(relative);
        scan.record_if_match(display);
        Ok(())
    }

    async fn canonical_entry(&self, path: &Path) -> Result<PathBuf, ToolExecutionError> {
        let canonical = self
            .file_system
            .canonicalize(path.to_path_buf())
            .await
            .map_err(|error| io_error(path.display(), error, "glob"))?;
        if !is_inside(&self.root, &canonical) {
            return Err(path_outside_workspace(&path.to_string_lossy(), "glob"));
        }
        Ok(canonical)
    }

    async fn read_directory(&self, path: PathBuf) -> Result<DirectoryEntries, ToolExecutionError> {
        self.file_system
            .read_directory(path.clone())
            .await
            .map_err(|error| io_error(path.display(), error, "glob"))
    }

    async fn path_kind(&self, path: PathBuf) -> Result<PathKind, ToolExecutionError> {
        self.file_system
            .path_kind(path.clone())
            .await
            .map_err(|error| io_error(path.display(), error, "glob"))
    }
}

impl ToolPermissionMetadata for GlobFiles {
    fn permission_requirement(&self) -> PermissionRequirement {
        self.permission
    }
}

impl Tool for GlobFiles {
    const NAME: &'static str = "glob_files";
    type Args = GlobFilesArgs;
    type Output = GlobFilesOutput;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Find regular workspace files by a relative glob pattern using *, **, and ?. Use this for filename/path discovery, not content search; use search_files for literal text.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "pattern": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Relative glob pattern for file paths, for example src/**/*.rs."
                }
            },
            "required": ["pattern"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        self.glob_path(&args.pattern).await
    }
}

#[cfg(test)]
mod tests {
    use rig::tool::{Tool, ToolExecutionError};

    use super::super::scan::MAX_MATCHES;
    use super::{GlobFiles, TokioFileSystem};
    use crate::shared::tool_permissions::catalog::{PermissionRequirement, ToolPermissionMetadata};
    use crate::tools::{
        error_codes,
        tool_glob_files::{pattern::GlobPattern, scan::GlobScan},
        utils::path::MAX_PATH_BYTES,
    };

    fn pattern(value: &str) -> Option<GlobPattern> {
        GlobPattern::parse(value).ok()
    }

    #[test]
    fn schema_has_only_one_required_pattern() {
        let tool = GlobFiles {
            root: std::path::PathBuf::from("/workspace"),
            file_system: std::sync::Arc::new(TokioFileSystem),
            permission: PermissionRequirement::Automatic,
        };
        let schema = tool.parameters();

        assert_eq!(schema["required"], serde_json::json!(["pattern"]));
        assert_eq!(schema["additionalProperties"], serde_json::json!(false));
        assert_eq!(schema["properties"]["pattern"]["minLength"], 1);
        assert_eq!(
            schema["properties"].as_object().map(|value| value.len()),
            Some(1)
        );
    }

    #[test]
    fn glob_files_is_automatic() {
        let tool = GlobFiles {
            root: std::path::PathBuf::from("/workspace"),
            file_system: std::sync::Arc::new(TokioFileSystem),
            permission: PermissionRequirement::Automatic,
        };

        assert_eq!(
            tool.permission_requirement(),
            PermissionRequirement::Automatic
        );
    }

    #[test]
    fn normalizes_separators_and_matches_recursive_paths() {
        let parsed = pattern(r"src\**\*.rs");
        assert_eq!(
            parsed.as_ref().map(GlobPattern::normalized),
            Some("src/**/*.rs")
        );
        assert!(parsed.is_some_and(|value| value.matches("src/main.rs")));
        assert!(pattern("src/**/*.rs").is_some_and(|value| value.matches("src/tools/mod.rs")));
        assert!(!pattern("src/*.rs").is_some_and(|value| value.matches("src/tools/mod.rs")));
        assert!(pattern("src/?ain.rs").is_some_and(|value| value.matches("src/main.rs")));
    }

    #[test]
    fn rejects_invalid_patterns_before_filesystem_access() {
        let oversized = "x".repeat(MAX_PATH_BYTES + 1);
        for value in [
            "",
            "bad\0pattern",
            "/tmp/*.rs",
            "C:/tmp/*.rs",
            "../*.rs",
            "src/../*.rs",
            "src/[a].rs",
        ] {
            let error = GlobPattern::parse(value).err();
            assert!(error.is_some(), "pattern should be rejected: {value:?}");
        }
        assert!(GlobPattern::parse(&oversized).is_err());
        assert_eq!(
            GlobPattern::parse("../*.rs")
                .err()
                .as_ref()
                .and_then(ToolExecutionError::code),
            Some(error_codes::PATH_OUTSIDE_WORKSPACE)
        );
    }

    #[test]
    fn output_is_sorted_unique_and_no_match_is_success() {
        let Some(parsed) = pattern("src/**/*.rs") else {
            return;
        };
        let mut scan = GlobScan::new(parsed);
        scan.record_if_match("src/z.rs".to_string());
        scan.record_if_match("src/a.rs".to_string());
        scan.record_if_match("src/a.rs".to_string());
        let output = scan.output();
        assert_eq!(output.files, vec!["src/a.rs", "src/z.rs"]);
        assert_eq!(output.match_count, 2);
        assert!(!output.truncated);
        assert_eq!(
            serde_json::to_value(&output).ok(),
            Some(serde_json::json!({
                "action": "matched",
                "ok": true,
                "pattern": "src/**/*.rs",
                "files": ["src/a.rs", "src/z.rs"],
                "match_count": 2,
                "truncated": false
            }))
        );

        let Some(parsed) = pattern("missing") else {
            return;
        };
        let empty = GlobScan::new(parsed).output();
        assert!(empty.ok);
        assert!(empty.files.is_empty());
        assert_eq!(empty.match_count, 0);
        assert!(!empty.truncated);
        assert_eq!(
            serde_json::to_value(&empty).ok(),
            Some(serde_json::json!({
                "action": "matched",
                "ok": true,
                "pattern": "missing",
                "files": [],
                "match_count": 0,
                "truncated": false
            }))
        );
    }

    #[test]
    fn result_cap_marks_output_truncated() {
        let Some(parsed) = pattern("**/*.rs") else {
            return;
        };
        let mut scan = GlobScan::new(parsed);
        for index in 0..=MAX_MATCHES {
            scan.record_if_match(format!("src/{index:03}.rs"));
        }
        let output = scan.output();
        assert_eq!(output.match_count, MAX_MATCHES);
        assert_eq!(output.files.len(), MAX_MATCHES);
        assert!(output.truncated);
        assert!(output.message.is_some());
    }
}
