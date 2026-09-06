use std::{
    env,
    path::{Path, PathBuf},
    sync::Arc,
};

use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::{Deserialize, Serialize};

use super::{
    diff::bounded_diff,
    errors::{file_access_error, io_error, stale_content_error, write_error},
    filesystem::{FileSystem, TokioFileSystem},
    validation::{
        MAX_FILE_BYTES, file_too_large_error, path_outside_workspace, replaced_text, unique_match,
        unsupported_file_error, validate_arguments, validate_path, validate_result_size,
    },
};
use crate::shared::tool_permissions::catalog::{PermissionRequirement, ToolPermissionMetadata};
use crate::tools::action::Action;

#[cfg(test)]
use super::diff::{TEST_MAX_DIFF_BYTES, changed_line_count};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EditFileArgs {
    path: String,
    old_text: String,
    new_text: String,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct EditFileOutput {
    action: Action,
    ok: bool,
    path: String,
    replacements: usize,
    changed: bool,
    diff: String,
    truncated: bool,
}

/// Performs one exact, bounded replacement in an existing workspace text file.
pub(crate) struct EditFile {
    root: PathBuf,
    file_system: Arc<dyn FileSystem>,
    permission: PermissionRequirement,
}

impl EditFile {
    /// Creates an edit tool rooted at Rigel's canonical startup workspace.
    pub(crate) async fn new() -> Result<Self, ToolExecutionError> {
        let file_system: Arc<dyn FileSystem> = Arc::new(TokioFileSystem);
        let current_dir = env::current_dir().map_err(|error| io_error("workspace", error))?;
        let root = file_system
            .canonicalize(current_dir.clone())
            .await
            .map_err(|error| io_error(current_dir.display(), error))?;
        Ok(Self {
            root,
            file_system,
            permission: PermissionRequirement::ConfirmationRequired,
        })
    }

    async fn edit_path(
        &self,
        requested_path: &str,
        old_text: &str,
        new_text: &str,
    ) -> Result<EditFileOutput, ToolExecutionError> {
        validate_arguments(requested_path, old_text, new_text)?;
        let candidate = self.root.join(validate_path(requested_path)?);
        let canonical = self.canonical_path(&candidate, requested_path).await?;
        self.ensure_regular_file(&canonical, requested_path).await?;
        let original = self.read_text(&canonical, requested_path).await?;
        let (start, end) = unique_match(&original, old_text)?;
        let current = self.read_bytes(&canonical, requested_path).await?;
        if current != original.as_bytes() {
            return Err(stale_content_error(requested_path));
        }
        let updated = replaced_text(&original, start, end, new_text);
        validate_result_size(updated.len())?;
        let path = crate::tools::tool_search_files::workspace::display_path(
            &self.root,
            &canonical,
            requested_path,
        )?;
        let diff = bounded_diff(&path, &original, &updated, start, old_text, new_text);
        self.file_system
            .write(canonical, updated.into_bytes())
            .await
            .map_err(|error| write_error(requested_path, error))?;
        Ok(EditFileOutput {
            action: Action::Edited,
            ok: true,
            path,
            replacements: 1,
            changed: true,
            diff: diff.content,
            truncated: diff.truncated,
        })
    }

    async fn canonical_path(
        &self,
        candidate: &Path,
        requested_path: &str,
    ) -> Result<PathBuf, ToolExecutionError> {
        let canonical = self
            .file_system
            .canonicalize(candidate.to_path_buf())
            .await
            .map_err(|error| file_access_error(requested_path, error))?;
        if !canonical.starts_with(&self.root) {
            return Err(path_outside_workspace(requested_path));
        }
        Ok(canonical)
    }

    async fn ensure_regular_file(
        &self,
        path: &Path,
        requested_path: &str,
    ) -> Result<(), ToolExecutionError> {
        let is_file = self
            .file_system
            .metadata(path.to_path_buf())
            .await
            .map_err(|error| file_access_error(requested_path, error))?;
        if !is_file {
            return Err(unsupported_file_error(requested_path));
        }
        Ok(())
    }

    async fn read_text(
        &self,
        path: &Path,
        requested_path: &str,
    ) -> Result<String, ToolExecutionError> {
        let bytes = self.read_bytes(path, requested_path).await?;
        crate::tools::tool_search_files::search::decode_text(bytes)
            .ok_or_else(|| unsupported_file_error(requested_path))
    }

    async fn read_bytes(
        &self,
        path: &Path,
        requested_path: &str,
    ) -> Result<Vec<u8>, ToolExecutionError> {
        let bytes = self
            .file_system
            .read(path.to_path_buf())
            .await
            .map_err(|error| file_access_error(requested_path, error))?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err(file_too_large_error(requested_path));
        }
        Ok(bytes)
    }
}

impl ToolPermissionMetadata for EditFile {
    fn permission_requirement(&self) -> PermissionRequirement {
        self.permission
    }
}

impl Tool for EditFile {
    const NAME: &'static str = "edit_file";
    type Args = EditFileArgs;
    type Output = EditFileOutput;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Safely replace one exact, unique text fragment in an existing UTF-8 workspace file. Read the file first; this tool requires confirmation and returns a bounded diff.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "path": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Relative path of the text file to edit."
                },
                "old_text": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Exact text that must occur exactly once in the current file."
                },
                "new_text": {
                    "type": "string",
                    "description": "Replacement text; an empty string deletes the exact match."
                }
            },
            "required": ["path", "old_text", "new_text"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        self.edit_path(&args.path, &args.old_text, &args.new_text)
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        io::ErrorKind,
        path::PathBuf,
        sync::{Arc, Mutex},
    };

    use rig::tool::{Tool, ToolContext};

    use super::{
        EditFile, EditFileArgs, FileSystem, TEST_MAX_DIFF_BYTES, bounded_diff, changed_line_count,
        replaced_text, unique_match,
    };
    use crate::shared::tool_permissions::catalog::{PermissionRequirement, ToolPermissionMetadata};
    use crate::tools::error_codes;
    use crate::tools::tool_edit_file::validation::MAX_EDIT_TEXT_BYTES;

    struct FakeFileSystem {
        canonical_path: Result<PathBuf, ErrorKind>,
        metadata: Result<bool, ErrorKind>,
        reads: Mutex<VecDeque<Result<Vec<u8>, ErrorKind>>>,
        write_result: Mutex<Result<(), ErrorKind>>,
        writes: Mutex<Vec<Vec<u8>>>,
    }

    impl FakeFileSystem {
        fn new(bytes: Vec<Vec<u8>>) -> Self {
            Self {
                canonical_path: Ok(PathBuf::from("/workspace/src/main.rs")),
                metadata: Ok(true),
                reads: Mutex::new(bytes.into_iter().map(Ok).collect()),
                write_result: Mutex::new(Ok(())),
                writes: Mutex::new(Vec::new()),
            }
        }

        fn failing_read(error: ErrorKind) -> Self {
            Self {
                canonical_path: Err(error),
                metadata: Ok(true),
                reads: Mutex::new(VecDeque::new()),
                write_result: Mutex::new(Ok(())),
                writes: Mutex::new(Vec::new()),
            }
        }

        fn with_write_error(self, error: ErrorKind) -> Self {
            if let Ok(mut result) = self.write_result.lock() {
                *result = Err(error);
            }
            self
        }
    }

    impl FileSystem for FakeFileSystem {
        fn canonicalize(
            &self,
            _path: PathBuf,
        ) -> futures::future::BoxFuture<'static, std::io::Result<PathBuf>> {
            let result = self.canonical_path.clone().map_err(std::io::Error::from);
            Box::pin(async move { result })
        }

        fn metadata(
            &self,
            _path: PathBuf,
        ) -> futures::future::BoxFuture<'static, std::io::Result<bool>> {
            let result = self.metadata.map_err(std::io::Error::from);
            Box::pin(async move { result })
        }

        fn read(
            &self,
            _path: PathBuf,
        ) -> futures::future::BoxFuture<'static, std::io::Result<Vec<u8>>> {
            let result = self
                .reads
                .lock()
                .ok()
                .and_then(|mut reads| reads.pop_front())
                .unwrap_or(Err(ErrorKind::Other));
            Box::pin(async move { result.map_err(std::io::Error::from) })
        }

        fn write(
            &self,
            _path: PathBuf,
            contents: Vec<u8>,
        ) -> futures::future::BoxFuture<'static, std::io::Result<()>> {
            let result = self
                .write_result
                .lock()
                .map(|result| *result)
                .unwrap_or(Err(ErrorKind::Other));
            if result.is_ok()
                && let Ok(mut writes) = self.writes.lock()
            {
                writes.push(contents);
            }
            Box::pin(async move { result.map_err(std::io::Error::from) })
        }
    }

    fn tool(file_system: Arc<FakeFileSystem>) -> EditFile {
        EditFile {
            root: PathBuf::from("/workspace"),
            file_system,
            permission: PermissionRequirement::ConfirmationRequired,
        }
    }

    async fn edit(
        tool: &EditFile,
        old_text: &str,
        new_text: &str,
    ) -> Result<super::EditFileOutput, rig::tool::ToolExecutionError> {
        let mut context = ToolContext::default();
        tool.call(
            &mut context,
            EditFileArgs {
                path: "src/main.rs".to_string(),
                old_text: old_text.to_string(),
                new_text: new_text.to_string(),
            },
        )
        .await
    }

    #[test]
    fn schema_requires_exactly_three_scalar_arguments() {
        let file_system = Arc::new(FakeFileSystem::new(vec![b"old".to_vec(), b"old".to_vec()]));
        let schema = tool(file_system).parameters();

        assert_eq!(
            schema["required"],
            serde_json::json!(["path", "old_text", "new_text"])
        );
        assert_eq!(schema["additionalProperties"], serde_json::json!(false));
        assert_eq!(
            schema["properties"].as_object().map(|value| value.len()),
            Some(3)
        );
        assert_eq!(schema["properties"]["old_text"]["minLength"], 1);
        assert!(schema["properties"]["new_text"].get("minLength").is_none());
    }

    #[test]
    fn edit_file_requires_confirmation() {
        let file_system = Arc::new(FakeFileSystem::new(Vec::new()));
        assert_eq!(
            tool(file_system).permission_requirement(),
            PermissionRequirement::ConfirmationRequired
        );
    }

    #[tokio::test]
    async fn not_found_is_classified_without_writing() {
        let file_system = Arc::new(FakeFileSystem::failing_read(ErrorKind::NotFound));
        let result = edit(&tool(file_system), "old", "new").await;

        assert_eq!(
            result.err().as_ref().and_then(|error| error.code()),
            Some(error_codes::NOT_FOUND)
        );
    }

    #[tokio::test]
    async fn zero_and_multiple_matches_are_recoverable_errors() {
        let no_match = edit(
            &tool(Arc::new(FakeFileSystem::new(vec![b"other".to_vec()]))),
            "old",
            "new",
        )
        .await;
        let multiple = edit(
            &tool(Arc::new(FakeFileSystem::new(vec![b"old old".to_vec()]))),
            "old",
            "new",
        )
        .await;

        assert_eq!(
            no_match.err().as_ref().and_then(|error| error.code()),
            Some(error_codes::NO_MATCH)
        );
        assert_eq!(
            multiple.err().as_ref().and_then(|error| error.code()),
            Some(error_codes::AMBIGUOUS_MATCH)
        );
    }

    #[tokio::test]
    async fn stale_content_is_detected_before_write() {
        let file_system = Arc::new(FakeFileSystem::new(vec![
            b"old".to_vec(),
            b"changed".to_vec(),
        ]));
        let result = edit(&tool(file_system.clone()), "old", "new").await;

        assert_eq!(
            result.err().as_ref().and_then(|error| error.code()),
            Some(error_codes::STALE_CONTENT)
        );
        assert!(
            file_system
                .writes
                .lock()
                .ok()
                .is_some_and(|writes| writes.is_empty())
        );
    }

    #[tokio::test]
    async fn write_failure_is_an_error_without_a_success_result() {
        let file_system = Arc::new(
            FakeFileSystem::new(vec![b"old".to_vec(), b"old".to_vec()])
                .with_write_error(ErrorKind::PermissionDenied),
        );
        let result = edit(&tool(file_system.clone()), "old", "new").await;

        assert_eq!(
            result.err().as_ref().and_then(|error| error.code()),
            Some(error_codes::IO_ERROR)
        );
        assert!(
            file_system
                .writes
                .lock()
                .ok()
                .is_some_and(|writes| writes.is_empty())
        );
    }

    #[tokio::test]
    async fn no_op_is_rejected_before_reading() {
        let file_system = Arc::new(FakeFileSystem::new(Vec::new()));
        let result = edit(&tool(file_system), "old", "old").await;

        assert_eq!(
            result.err().as_ref().and_then(|error| error.code()),
            Some(error_codes::INVALID_ARGUMENT)
        );
    }

    #[tokio::test]
    async fn empty_replacement_deletes_the_unique_match() {
        let file_system = Arc::new(FakeFileSystem::new(vec![
            b"before old after".to_vec(),
            b"before old after".to_vec(),
        ]));
        let result = edit(&tool(file_system.clone()), "old", "").await;

        assert_eq!(
            result.as_ref().ok().map(|output| output.replacements),
            Some(1)
        );
        assert_eq!(
            file_system
                .writes
                .lock()
                .ok()
                .and_then(|writes| writes.first().cloned()),
            Some(b"before  after".to_vec())
        );
    }

    #[test]
    fn replacement_and_diff_remain_utf8_safe_and_bounded() {
        let (start, end) = match unique_match("α old ω", "old") {
            Ok(range) => range,
            Err(_) => return,
        };
        let updated = replaced_text("α old ω", start, end, "новый");
        let diff = bounded_diff("src/main.rs", "α old ω", &updated, start, "old", "новый");

        assert_eq!(updated, "α новый ω");
        assert!(updated.is_char_boundary(updated.len()));
        assert!(diff.content.contains("-α old ω"));
        assert!(diff.content.contains("+α новый ω"));
        assert_eq!(changed_line_count("old\n"), 1);
    }

    #[test]
    fn large_diff_is_truncated_with_an_explicit_marker() {
        let original = format!("{}old", "x".repeat(TEST_MAX_DIFF_BYTES));
        let updated = format!("{}new", "x".repeat(TEST_MAX_DIFF_BYTES));
        let diff = bounded_diff(
            "src/main.rs",
            &original,
            &updated,
            TEST_MAX_DIFF_BYTES,
            "old",
            "new",
        );

        assert!(diff.truncated);
        assert!(
            diff.content
                .ends_with("[diff truncated: output is bounded by server limits]")
        );
        assert!(diff.content.len() <= TEST_MAX_DIFF_BYTES);
    }

    #[test]
    fn oversized_edit_text_is_rejected_before_matching() {
        let error =
            super::validate_arguments("src/main.rs", &"x".repeat(MAX_EDIT_TEXT_BYTES + 1), "new");
        assert_eq!(
            error.err().as_ref().and_then(|value| value.code()),
            Some(error_codes::INVALID_ARGUMENT)
        );
    }
}
