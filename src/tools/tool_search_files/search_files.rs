use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::Deserialize;

use crate::{
    entities::{
        tool::tool_entities::PermissionRequirement,
        workspace::workspace_entities::{WorkspaceDirectoryEntries, WorkspacePathKind},
    },
    infrastructure::tools::{
        permissions::ToolPermissionMetadata,
        workspace::{WorkspacePathResolver, WorkspaceReader, WorkspaceScanner},
    },
    tools::utils::{
        errors::{path_outside_workspace, workspace_file_access_error},
        path::is_inside,
    },
};

use super::search::SearchFilesOutput;

#[cfg(test)]
const MAX_PATTERN_BYTES: usize = 4 * 1024;
const MAX_DIRECTORY_ENTRIES: usize = 2048;

/// Arguments for a literal, case-sensitive search rooted in the startup workspace.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SearchFilesArgs {
    pattern: String,
    path: String,
}

/// Read-only bounded content search with startup-workspace confinement.
pub(crate) struct SearchFiles {
    /// Canonical workspace directory used as the only permitted search root.
    pub(crate) root: PathBuf,
    /// Resolves tool paths through the composition-root-selected workspace.
    pub(super) path_resolver: Arc<dyn WorkspacePathResolver>,
    /// Reads bounded file content through the injected workspace.
    pub(super) reader: Arc<dyn WorkspaceReader>,
    /// Classifies and lists entries through the injected workspace.
    pub(super) scanner: Arc<dyn WorkspaceScanner>,
    /// Automatic permission because search is read-only and bounded.
    pub(crate) permission: PermissionRequirement,
}

impl SearchFiles {
    /// Canonicalizes a candidate through the injected root capability and enforces confinement.
    pub(super) async fn canonical_path(
        &self,
        candidate: &Path,
        requested_path: &str,
    ) -> Result<PathBuf, ToolExecutionError> {
        let canonical = self
            .path_resolver
            .canonicalize(candidate)
            .await
            .map_err(|error| workspace_file_access_error(requested_path, error, "search"))?;
        if !is_inside(&self.root, &canonical) {
            return Err(path_outside_workspace(requested_path, "search"));
        }
        Ok(canonical)
    }

    /// Classifies one path through the injected scanner and preserves tool error mapping.
    pub(super) async fn path_kind(
        &self,
        path: &Path,
    ) -> Result<WorkspacePathKind, ToolExecutionError> {
        self.scanner
            .path_kind(path)
            .await
            .map_err(|error| workspace_file_access_error(path.display(), error, "search"))
    }

    /// Reads one bounded directory listing through the injected scanner.
    pub(super) async fn read_directory(
        &self,
        path: &Path,
    ) -> Result<WorkspaceDirectoryEntries, ToolExecutionError> {
        self.scanner
            .read_directory(path, MAX_DIRECTORY_ENTRIES)
            .await
            .map_err(|error| workspace_file_access_error(path.display(), error, "search"))
    }
}

impl ToolPermissionMetadata for SearchFiles {
    fn permission_requirement(&self) -> PermissionRequirement {
        self.permission
    }
}

impl Tool for SearchFiles {
    const NAME: &'static str = "search_files";
    type Args = SearchFilesArgs;
    type Output = SearchFilesOutput;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Search literal, case-sensitive text in bounded UTF-8 workspace files. Use read_file to inspect a matching file; this is content search, not filename discovery.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "pattern": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Literal text to search for."
                },
                "path": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Relative file or directory path in which to search."
                }
            },
            "required": ["pattern", "path"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        self.search_path(&args.pattern, &args.path).await
    }
}

#[cfg(test)]
mod tests {
    use std::{
        path::{Path, PathBuf},
        sync::Arc,
    };

    use rig::tool::Tool;

    use super::{MAX_PATTERN_BYTES, SearchFiles};
    use crate::{
        entities::tool::tool_entities::PermissionRequirement,
        entities::workspace::workspace_entities::{
            WorkspaceDirectoryEntries, WorkspacePathKind, WorkspaceRead,
        },
        infrastructure::tools::{
            permissions::ToolPermissionMetadata,
            workspace::{WorkspacePathResolver, WorkspaceReader, WorkspaceScanner},
        },
        tools::{
            error_codes,
            tool_search_files::search::{MAX_MATCHES, SearchContext},
            utils::{
                path::{MAX_PATH_BYTES, validate_relative_path},
                text::utf8_prefix,
            },
        },
    };

    type WorkspaceFuture<'a, T> =
        std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<T>> + Send + 'a>>;

    struct FakeWorkspace;

    impl WorkspacePathResolver for FakeWorkspace {
        fn root(&self) -> PathBuf {
            PathBuf::from("/workspace")
        }

        fn resolve(&self, path: &str) -> anyhow::Result<PathBuf> {
            Ok(self.root().join(path))
        }

        fn canonicalize(&self, path: &Path) -> WorkspaceFuture<'_, PathBuf> {
            let path = path.to_path_buf();
            Box::pin(async move { Ok(path) })
        }
    }

    impl WorkspaceReader for FakeWorkspace {
        fn metadata(&self, _path: &Path) -> WorkspaceFuture<'_, WorkspacePathKind> {
            Box::pin(async { Ok(WorkspacePathKind::File) })
        }

        fn read(&self, _path: &Path, _max_bytes: usize) -> WorkspaceFuture<'_, WorkspaceRead> {
            Box::pin(async {
                Ok(WorkspaceRead {
                    bytes: Vec::new(),
                    truncated: false,
                })
            })
        }
    }

    impl WorkspaceScanner for FakeWorkspace {
        fn path_kind(&self, _path: &Path) -> WorkspaceFuture<'_, WorkspacePathKind> {
            Box::pin(async { Ok(WorkspacePathKind::Directory) })
        }

        fn read_directory(
            &self,
            _path: &Path,
            _max_entries: usize,
        ) -> WorkspaceFuture<'_, WorkspaceDirectoryEntries> {
            Box::pin(async {
                Ok(WorkspaceDirectoryEntries {
                    paths: Vec::new(),
                    truncated: false,
                })
            })
        }
    }

    fn tool() -> SearchFiles {
        let workspace = Arc::new(FakeWorkspace);
        SearchFiles {
            root: PathBuf::from("/workspace"),
            path_resolver: workspace.clone(),
            reader: workspace.clone(),
            scanner: workspace,
            permission: PermissionRequirement::Automatic,
        }
    }

    fn context(pattern: &str) -> SearchContext {
        SearchContext::new(PathBuf::from("/workspace"), pattern)
    }

    #[test]
    fn schema_requires_only_pattern_and_path() {
        let schema = tool().parameters();

        assert_eq!(schema["required"], serde_json::json!(["pattern", "path"]));
        assert_eq!(schema["additionalProperties"], serde_json::json!(false));
        assert_eq!(schema["properties"]["pattern"]["minLength"], 1);
        assert_eq!(schema["properties"]["path"]["minLength"], 1);
        assert_eq!(
            schema["properties"].as_object().map(|value| value.len()),
            Some(2)
        );
    }

    #[test]
    fn search_files_is_automatic() {
        assert_eq!(
            tool().permission_requirement(),
            PermissionRequirement::Automatic
        );
    }

    #[test]
    fn literal_search_is_case_sensitive_and_reports_one_based_lines() {
        let mut context = context("Error");
        context.record_matches("src/main.rs", "ok\nError here\nerror again");
        let value = serde_json::to_value(context.output("src".to_string())).ok();

        assert_eq!(
            value,
            Some(serde_json::json!({
                "action": "searched",
                "ok": true,
                "pattern": "Error",
                "path": "src",
                "matches": [{"path": "src/main.rs", "line": 2, "content": "Error here"}],
                "match_count": 1,
                "truncated": false,
                "next_offset": null
            }))
        );
    }

    #[test]
    fn no_match_is_a_success_with_an_empty_array() {
        let mut context = context("missing");
        context.record_matches("src/main.rs", "present");
        let output = serde_json::to_value(context.output("src".to_string())).ok();

        assert_eq!(
            output.as_ref().and_then(|value| value.get("matches")),
            Some(&serde_json::json!([]))
        );
        assert_eq!(
            output.as_ref().and_then(|value| value.get("match_count")),
            Some(&serde_json::json!(0))
        );
        assert_eq!(
            output.as_ref().and_then(|value| value.get("truncated")),
            Some(&serde_json::json!(false))
        );
    }

    #[test]
    fn match_limit_marks_output_truncated() {
        let mut context = context("hit");
        let text = (0..=MAX_MATCHES)
            .map(|line| format!("hit {line}"))
            .collect::<Vec<_>>()
            .join("\n");

        context.record_matches("large.txt", &text);

        assert_eq!(context.match_count(), MAX_MATCHES);
        assert!(context.is_truncated());
        assert!(context.should_stop());
    }

    #[test]
    fn invalid_paths_are_rejected_without_filesystem_access() {
        for path in ["", " ", "../outside", "/etc/passwd", "bad\0path"] {
            let result = validate_relative_path(path, MAX_PATH_BYTES, "search");
            assert!(result.is_err(), "path should be rejected: {path:?}");
        }
        assert_eq!(
            validate_relative_path("/etc/passwd", MAX_PATH_BYTES, "search")
                .err()
                .as_ref()
                .and_then(|error| error.code()),
            Some(error_codes::PATH_OUTSIDE_WORKSPACE)
        );
    }

    #[test]
    fn pattern_validation_rejects_empty_nul_and_oversized_values() {
        for pattern in ["", "bad\0pattern", &"x".repeat(MAX_PATTERN_BYTES + 1)] {
            assert!(SearchFiles::validate_pattern(pattern).is_err());
        }
    }

    #[test]
    fn utf8_limits_end_at_character_boundaries() {
        let text = "я".repeat(10);
        let end = utf8_prefix(&text, 3);

        assert!(text.is_char_boundary(end));
        assert!(end <= 3);
    }
}
