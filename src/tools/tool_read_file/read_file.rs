use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::{Deserialize, Serialize};

use crate::{
    entities::{
        tool::tool_entities::PermissionRequirement,
        workspace::workspace_entities::WorkspacePathKind,
    },
    infrastructure::tools::{
        permissions::ToolPermissionMetadata,
        workspace::{WorkspacePathResolver, WorkspaceReader},
    },
};

use crate::tools::{
    action::Action,
    error_codes,
    utils::{
        errors::{path_outside_workspace, workspace_file_access_error},
        path::{display_workspace_path, is_inside, validate_non_absolute_path},
        text::decode_text,
    },
};
const MAX_CONTENT_BYTES: usize = 32 * 1024;
const MAX_LINES: usize = 200;
const TRUNCATION_MESSAGE: &str = "Content is truncated. Call read_file again only with a separate bounded range tool after that tool becomes available; do not use shell pipelines for ordinary file reading.";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadFileArgs {
    path: String,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ReadFileOutput {
    action: Action,
    ok: bool,
    path: String,
    content: String,
    start_line: usize,
    end_line: usize,
    total_lines: usize,
    truncated: bool,
    next_offset: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

pub(crate) struct ReadFile {
    root: PathBuf,
    path_resolver: Arc<dyn WorkspacePathResolver>,
    reader: Arc<dyn WorkspaceReader>,
    permission: PermissionRequirement,
}

impl ReadFile {
    /// Creates a read tool from an injected root resolver and bounded reader.
    pub(crate) async fn new(
        path_resolver: Arc<dyn WorkspacePathResolver>,
        reader: Arc<dyn WorkspaceReader>,
    ) -> Result<Self, ToolExecutionError> {
        let root = path_resolver.root();
        Ok(Self {
            root,
            path_resolver,
            reader,
            permission: PermissionRequirement::Automatic,
        })
    }

    async fn read_path(&self, requested_path: &str) -> Result<ReadFileOutput, ToolExecutionError> {
        Self::validate_path(requested_path)?;
        let candidate = self
            .path_resolver
            .resolve(requested_path)
            .map_err(|error| workspace_file_access_error(requested_path, error, "read"))?;
        let canonical_path = self.canonical_path(&candidate, requested_path).await?;
        self.ensure_regular_file(&canonical_path, requested_path)
            .await?;
        let read = self
            .reader
            .read(&canonical_path, MAX_CONTENT_BYTES)
            .await
            .map_err(|error| workspace_file_access_error(requested_path, error, "read"))?;
        let text = decode_text(read.bytes).ok_or_else(|| unsupported_file_error(requested_path))?;
        let display_path =
            display_workspace_path(&self.root, &canonical_path, requested_path, "read")?;
        let mut output = build_output(display_path, &text);
        if read.truncated {
            mark_byte_truncation(&mut output);
        }
        Ok(output)
    }

    fn validate_path(path: &str) -> Result<PathBuf, ToolExecutionError> {
        validate_non_absolute_path(path, "read")
    }

    async fn canonical_path(
        &self,
        candidate: &Path,
        requested_path: &str,
    ) -> Result<PathBuf, ToolExecutionError> {
        let canonical_path = self
            .path_resolver
            .canonicalize(candidate)
            .await
            .map_err(|error| workspace_file_access_error(requested_path, error, "read"))?;
        if !is_inside(&self.root, &canonical_path) {
            return Err(path_outside_workspace(requested_path, "read"));
        }
        Ok(canonical_path)
    }

    async fn ensure_regular_file(
        &self,
        path: &Path,
        requested_path: &str,
    ) -> Result<(), ToolExecutionError> {
        let metadata = self
            .reader
            .metadata(path)
            .await
            .map_err(|error| workspace_file_access_error(requested_path, error, "read"))?;
        if metadata != WorkspacePathKind::File {
            return Err(ToolExecutionError::invalid_args(format!(
                "Cannot read file \"{requested_path}\": the path is not a regular file. Choose a UTF-8 text file.",
            ))
            .with_code(error_codes::INVALID_ARGUMENT));
        }
        Ok(())
    }
}

impl ToolPermissionMetadata for ReadFile {
    fn permission_requirement(&self) -> PermissionRequirement {
        self.permission
    }
}

impl Tool for ReadFile {
    const NAME: &'static str = "read_file";
    type Args = ReadFileArgs;
    type Output = ReadFileOutput;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Read a bounded UTF-8 text file from the workspace. Prefer this over run_command for ordinary file reading; returned lines are numbered from 1.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "path": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Relative path of the text file to read."
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
        self.read_path(&args.path).await
    }
}

fn build_output(path: String, text: &str) -> ReadFileOutput {
    let bounded = bounded_content(text);
    ReadFileOutput {
        action: Action::Read,
        ok: true,
        path,
        content: bounded.content,
        start_line: 1,
        end_line: bounded.end_line,
        total_lines: bounded.total_lines,
        truncated: bounded.truncated,
        next_offset: bounded.next_offset,
        message: bounded.message,
    }
}

fn mark_byte_truncation(output: &mut ReadFileOutput) {
    output.truncated = true;
    output.next_offset = Some(output.end_line.saturating_add(1));
    output.message = Some(TRUNCATION_MESSAGE.to_string());
}

struct BoundedContent {
    content: String,
    end_line: usize,
    total_lines: usize,
    truncated: bool,
    next_offset: Option<usize>,
    message: Option<String>,
}
fn bounded_content(text: &str) -> BoundedContent {
    let lines = text.lines().collect::<Vec<_>>();
    let total_lines = lines.len();
    let mut content = String::new();
    let mut end_line = 0;
    for (index, line) in lines.iter().enumerate().take(MAX_LINES) {
        let numbered_line = format!("{}  {line}", index + 1);
        let separator_bytes = usize::from(!content.is_empty());
        if content
            .len()
            .saturating_add(separator_bytes)
            .saturating_add(numbered_line.len())
            > MAX_CONTENT_BYTES
        {
            break;
        }
        if !content.is_empty() {
            content.push('\n');
        }
        content.push_str(&numbered_line);
        end_line = index + 1;
    }
    let truncated = end_line < total_lines;
    BoundedContent {
        content,
        end_line,
        total_lines,
        truncated,
        next_offset: truncated.then_some(end_line + 1),
        message: truncated.then(|| TRUNCATION_MESSAGE.to_string()),
    }
}

fn unsupported_file_error(path: &str) -> ToolExecutionError {
    ToolExecutionError::other(format!(
        "Cannot read file \"{path}\": only ordinary UTF-8 text files are supported; binary files are rejected.",
    ))
    .with_code(error_codes::UNSUPPORTED_FILE)
}

#[cfg(test)]
mod tests {
    use std::{io::ErrorKind, path::PathBuf, sync::Arc};

    use rig::tool::{Tool, ToolContext};

    use super::{
        MAX_CONTENT_BYTES, MAX_LINES, ReadFile, ReadFileArgs, ReadFileOutput, bounded_content,
    };
    use crate::{
        entities::tool::tool_entities::PermissionRequirement,
        entities::workspace::workspace_entities::{WorkspacePathKind, WorkspaceRead},
        infrastructure::tools::{
            permissions::ToolPermissionMetadata,
            workspace::{WorkspacePathResolver, WorkspaceReader},
        },
        tools::error_codes,
    };

    type WorkspaceFuture<'a, T> =
        std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<T>> + Send + 'a>>;

    #[derive(Clone)]
    struct FakeFileSystem {
        canonical_path: Result<PathBuf, ErrorKind>,
        metadata: Result<bool, ErrorKind>,
        bytes: Result<Vec<u8>, ErrorKind>,
    }

    impl FakeFileSystem {
        fn new(
            canonical_path: Result<PathBuf, ErrorKind>,
            metadata: Result<bool, ErrorKind>,
            bytes: Result<Vec<u8>, ErrorKind>,
        ) -> Self {
            Self {
                canonical_path,
                metadata,
                bytes,
            }
        }
    }

    impl WorkspacePathResolver for FakeFileSystem {
        fn root(&self) -> PathBuf {
            PathBuf::from("/workspace")
        }

        fn resolve(&self, path: &str) -> anyhow::Result<PathBuf> {
            Ok(self.root().join(path))
        }

        fn canonicalize(&self, _path: &std::path::Path) -> WorkspaceFuture<'_, PathBuf> {
            let result = self.canonical_path.clone().map_err(std::io::Error::from);
            Box::pin(async move { result.map_err(anyhow::Error::from) })
        }
    }

    impl WorkspaceReader for FakeFileSystem {
        fn metadata(&self, _path: &std::path::Path) -> WorkspaceFuture<'_, WorkspacePathKind> {
            let result = self
                .metadata
                .map(|is_file| {
                    if is_file {
                        WorkspacePathKind::File
                    } else {
                        WorkspacePathKind::Directory
                    }
                })
                .map_err(std::io::Error::from);
            Box::pin(async move { result.map_err(anyhow::Error::from) })
        }

        fn read(
            &self,
            _path: &std::path::Path,
            _max_bytes: usize,
        ) -> WorkspaceFuture<'_, WorkspaceRead> {
            let result = self
                .bytes
                .clone()
                .map(|bytes| WorkspaceRead {
                    bytes,
                    truncated: false,
                })
                .map_err(std::io::Error::from);
            Box::pin(async move { result.map_err(anyhow::Error::from) })
        }
    }

    fn tool(file_system: FakeFileSystem) -> ReadFile {
        let file_system = Arc::new(file_system);
        ReadFile {
            root: PathBuf::from("/workspace"),
            path_resolver: file_system.clone(),
            reader: file_system,
            permission: PermissionRequirement::Automatic,
        }
    }

    fn regular_file(bytes: Vec<u8>) -> ReadFile {
        tool(FakeFileSystem::new(
            Ok(PathBuf::from("/workspace/src/main.rs")),
            Ok(true),
            Ok(bytes),
        ))
    }

    async fn read(
        tool: &ReadFile,
        path: &str,
    ) -> Result<ReadFileOutput, rig::tool::ToolExecutionError> {
        let mut context = ToolContext::default();
        tool.call(
            &mut context,
            ReadFileArgs {
                path: path.to_string(),
            },
        )
        .await
    }

    #[test]
    fn schema_requires_only_path() {
        let schema = regular_file(Vec::new()).parameters();

        assert_eq!(schema["required"], serde_json::json!(["path"]));
        assert_eq!(schema["additionalProperties"], serde_json::json!(false));
        assert_eq!(schema["properties"]["path"]["minLength"], 1);
        assert_eq!(
            schema["properties"].as_object().map(|value| value.len()),
            Some(1)
        );
    }

    #[test]
    fn read_file_is_automatic() {
        let tool = regular_file(Vec::new());

        assert_eq!(
            tool.permission_requirement(),
            PermissionRequirement::Automatic
        );
    }

    #[test]
    fn numbered_content_uses_one_based_lines() {
        let content = bounded_content("first\nsecond");

        assert_eq!(content.content, "1  first\n2  second");
        assert_eq!(content.end_line, 2);
        assert_eq!(content.total_lines, 2);
        assert!(!content.truncated);
        assert_eq!(content.next_offset, None);
    }

    #[test]
    fn byte_limit_truncates_before_a_line_that_does_not_fit() {
        let text = (0..MAX_LINES)
            .map(|_| "x".repeat(MAX_CONTENT_BYTES / 100))
            .collect::<Vec<_>>()
            .join("\n");
        let content = bounded_content(&text);

        assert!(content.truncated);
        assert!(content.content.len() <= MAX_CONTENT_BYTES);
        assert_eq!(content.next_offset, Some(content.end_line + 1));
        assert!(content.end_line < MAX_LINES);
    }

    #[test]
    fn line_limit_reports_the_first_unreturned_line() {
        let text = (0..=MAX_LINES)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let content = bounded_content(&text);

        assert_eq!(content.total_lines, MAX_LINES + 1);
        assert_eq!(content.end_line, MAX_LINES);
        assert_eq!(content.next_offset, Some(MAX_LINES + 1));
        assert!(
            content
                .message
                .is_some_and(|message| message.contains("range tool"))
        );
    }

    #[tokio::test]
    async fn successful_read_returns_normalized_bounded_output() {
        let output = read(
            &regular_file(b"first\nsecond".to_vec()),
            "src/../src/main.rs",
        )
        .await
        .ok();

        assert_eq!(
            output.map(|value| serde_json::to_value(value).ok()),
            Some(Some(serde_json::json!({
                "action": "read",
                "ok": true,
                "path": "src/main.rs",
                "content": "1  first\n2  second",
                "start_line": 1,
                "end_line": 2,
                "total_lines": 2,
                "truncated": false,
                "next_offset": null
            })))
        );
    }

    #[tokio::test]
    async fn blank_and_nul_paths_are_invalid_arguments() {
        for path in [" ", "bad\0path"] {
            let result = read(&regular_file(Vec::new()), path).await;

            assert!(result.is_err());
            if let Err(error) = result {
                assert_eq!(error.code(), Some(error_codes::INVALID_ARGUMENT));
            }
        }
    }

    #[tokio::test]
    async fn absolute_and_escaping_paths_are_rejected() {
        let absolute = read(&regular_file(Vec::new()), "/etc/passwd").await;
        let escaping = read(
            &tool(FakeFileSystem::new(
                Ok(PathBuf::from("/outside/secret")),
                Ok(true),
                Ok(Vec::new()),
            )),
            "link",
        )
        .await;

        for result in [absolute, escaping] {
            assert!(result.is_err());
            if let Err(error) = result {
                assert_eq!(error.code(), Some(error_codes::PATH_OUTSIDE_WORKSPACE));
            }
        }
    }

    #[tokio::test]
    async fn missing_directory_binary_and_io_failures_are_classified() {
        let missing = read(
            &tool(FakeFileSystem::new(
                Err(ErrorKind::NotFound),
                Ok(true),
                Ok(Vec::new()),
            )),
            "missing.txt",
        )
        .await;
        let directory = read(
            &tool(FakeFileSystem::new(
                Ok(PathBuf::from("/workspace/src")),
                Ok(false),
                Ok(Vec::new()),
            )),
            "src",
        )
        .await;
        let binary = read(&regular_file(vec![0, 159, 146, 150]), "image.bin").await;
        let io_failure = read(
            &tool(FakeFileSystem::new(
                Ok(PathBuf::from("/workspace/src/main.rs")),
                Ok(true),
                Err(ErrorKind::PermissionDenied),
            )),
            "src/main.rs",
        )
        .await;

        assert!(missing.is_err());
        assert!(directory.is_err());
        assert!(binary.is_err());
        assert!(io_failure.is_err());
        if let Err(error) = missing {
            assert_eq!(error.code(), Some(error_codes::NOT_FOUND));
        }
        if let Err(error) = directory {
            assert_eq!(error.code(), Some(error_codes::INVALID_ARGUMENT));
        }
        if let Err(error) = binary {
            assert_eq!(error.code(), Some(error_codes::UNSUPPORTED_FILE));
        }
        if let Err(error) = io_failure {
            assert_eq!(error.code(), Some(error_codes::IO_ERROR));
        }
    }
}
