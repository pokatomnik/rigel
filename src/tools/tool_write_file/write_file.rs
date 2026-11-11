use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::{Deserialize, Serialize};

use super::{
    errors::{parent_error, write_error},
    validation::{display_path, validate_arguments, validate_path},
};
use crate::tools::{
    action::Action,
    utils::{
        errors::{
            invalid_argument, path_outside_workspace, workspace_file_access_error,
            workspace_io_error,
        },
        path::is_inside,
    },
};
use crate::{
    entities::{
        tool::tool_entities::PermissionRequirement,
        workspace::workspace_entities::WorkspacePathKind,
    },
    infrastructure::tools::{
        permissions::ToolPermissionMetadata,
        workspace::{WorkspacePathResolver, WorkspaceWriter},
    },
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WriteFileArgs {
    path: String,
    content: String,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct WriteFileOutput {
    action: Action,
    ok: bool,
    path: String,
    bytes_written: usize,
    overwritten: bool,
    parent_created: bool,
}

/// Creates or completely replaces one UTF-8 text file in the startup workspace.
pub(crate) struct WriteFile {
    root: PathBuf,
    path_resolver: Arc<dyn WorkspacePathResolver>,
    writer: Arc<dyn WorkspaceWriter>,
    permission: PermissionRequirement,
}

impl WriteFile {
    /// Creates a write tool from injected path and write capabilities.
    pub(crate) async fn new(
        path_resolver: Arc<dyn WorkspacePathResolver>,
        writer: Arc<dyn WorkspaceWriter>,
    ) -> Result<Self, ToolExecutionError> {
        let root = path_resolver.root();
        Ok(Self {
            root,
            path_resolver,
            writer,
            permission: PermissionRequirement::ConfirmationRequired,
        })
    }

    async fn write_path(
        &self,
        args: &WriteFileArgs,
    ) -> Result<WriteFileOutput, ToolExecutionError> {
        validate_arguments(&args.path, &args.content)?;
        let relative = validate_path(&args.path)?;
        let candidate = self
            .path_resolver
            .resolve(&args.path)
            .map_err(|error| workspace_file_access_error(&args.path, error, "write"))?;
        let overwritten = self.inspect_target(&candidate, &args.path).await?;
        let Some(parent) = candidate.parent() else {
            return Err(invalid_argument(
                "path must name a file inside the workspace.",
            ));
        };
        let parent_created = self.prepare_parent(parent, &args.path).await?;
        self.writer
            .write(&candidate, args.content.as_bytes())
            .await
            .map_err(|error| write_error(&args.path, workspace_io_error(error)))?;
        Ok(WriteFileOutput {
            action: Action::Written,
            ok: true,
            path: display_path(&relative),
            bytes_written: args.content.len(),
            overwritten,
            parent_created,
        })
    }

    async fn inspect_target(
        &self,
        candidate: &Path,
        requested_path: &str,
    ) -> Result<bool, ToolExecutionError> {
        let kind = match self
            .writer
            .metadata(candidate)
            .await
            .map_err(workspace_io_error)
        {
            Ok(kind) => kind,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(super::errors::file_access_error(requested_path, error)),
        };
        let canonical = self
            .path_resolver
            .canonicalize(candidate)
            .await
            .map_err(|error| workspace_file_access_error(requested_path, error, "write"))?;
        self.ensure_inside(&canonical, requested_path)?;
        match kind {
            WorkspacePathKind::File => Ok(true),
            WorkspacePathKind::Directory => Err(invalid_argument(format!(
                "Cannot write file \"{requested_path}\": the target is a directory. Choose a file path."
            ))),
            WorkspacePathKind::Symlink | WorkspacePathKind::Other => {
                Err(invalid_argument(format!(
                    "Cannot write file \"{requested_path}\": the target is not a regular file. Choose a text file path."
                )))
            }
        }
    }

    async fn prepare_parent(
        &self,
        parent: &Path,
        requested_path: &str,
    ) -> Result<bool, ToolExecutionError> {
        match self
            .path_resolver
            .canonicalize(parent)
            .await
            .map_err(workspace_io_error)
        {
            Ok(canonical) => {
                self.ensure_inside(&canonical, requested_path)?;
                Ok(false)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.ensure_existing_parent(parent, requested_path).await?;
                self.writer
                    .create_dir_all(parent)
                    .await
                    .map_err(|error| parent_error(requested_path, workspace_io_error(error)))?;
                let canonical = self
                    .path_resolver
                    .canonicalize(parent)
                    .await
                    .map_err(|error| parent_error(requested_path, workspace_io_error(error)))?;
                self.ensure_inside(&canonical, requested_path)?;
                Ok(true)
            }
            Err(error) => Err(parent_error(requested_path, error)),
        }
    }

    async fn ensure_existing_parent(
        &self,
        parent: &Path,
        requested_path: &str,
    ) -> Result<(), ToolExecutionError> {
        let mut existing = parent.to_path_buf();
        loop {
            match self
                .path_resolver
                .canonicalize(&existing)
                .await
                .map_err(workspace_io_error)
            {
                Ok(canonical) => return self.ensure_inside(&canonical, requested_path),
                Err(error) if error.kind() == io::ErrorKind::NotFound && existing != self.root => {
                    if !existing.pop() {
                        return Err(parent_error(requested_path, error));
                    }
                }
                Err(error) => return Err(parent_error(requested_path, error)),
            }
        }
    }

    fn ensure_inside(
        &self,
        canonical: &Path,
        requested_path: &str,
    ) -> Result<(), ToolExecutionError> {
        if is_inside(&self.root, canonical) {
            return Ok(());
        }
        Err(path_outside_workspace(requested_path, "write"))
    }
}

impl ToolPermissionMetadata for WriteFile {
    fn permission_requirement(&self) -> PermissionRequirement {
        self.permission
    }
}

impl Tool for WriteFile {
    const NAME: &'static str = "write_file";
    type Args = WriteFileArgs;
    type Output = WriteFileOutput;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Create a new UTF-8 text file or intentionally replace an existing one completely. Parent directories are created inside the workspace; this tool requires confirmation. Use edit_file for a targeted replacement.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "path": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Relative path of the text file to create or overwrite."
                },
                "content": {
                    "type": "string",
                    "description": "Complete UTF-8 content to write; this operation never appends."
                }
            },
            "required": ["path", "content"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        self.write_path(&args).await
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        io::ErrorKind,
        path::{Path, PathBuf},
        sync::{Arc, Mutex},
    };

    use rig::tool::{Tool, ToolContext};

    use super::{WriteFile, WriteFileArgs, WriteFileOutput};
    use crate::{
        entities::tool::tool_entities::PermissionRequirement,
        entities::workspace::workspace_entities::WorkspacePathKind,
        infrastructure::tools::{
            permissions::ToolPermissionMetadata,
            workspace::{WorkspacePathResolver, WorkspaceWriter},
        },
        tools::{
            action::Action,
            error_codes,
            tool_write_file::{
                errors::parent_error,
                validation::{MAX_CONTENT_BYTES, display_path, validate_arguments, validate_path},
            },
        },
    };

    type WorkspaceFuture<'a, T> =
        std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<T>> + Send + 'a>>;

    struct FakeFileSystem {
        canonical: Mutex<VecDeque<Result<PathBuf, ErrorKind>>>,
        metadata: Result<WorkspacePathKind, ErrorKind>,
        create_result: Result<(), ErrorKind>,
        write_result: Result<(), ErrorKind>,
        created: Mutex<Vec<PathBuf>>,
        writes: Mutex<Vec<(PathBuf, Vec<u8>)>>,
    }

    impl FakeFileSystem {
        fn new(
            canonical: Vec<Result<PathBuf, ErrorKind>>,
            metadata: Result<WorkspacePathKind, ErrorKind>,
        ) -> Self {
            Self {
                canonical: Mutex::new(canonical.into_iter().collect()),
                metadata,
                create_result: Ok(()),
                write_result: Ok(()),
                created: Mutex::new(Vec::new()),
                writes: Mutex::new(Vec::new()),
            }
        }

        fn with_write_error(mut self, error: ErrorKind) -> Self {
            self.write_result = Err(error);
            self
        }
    }

    impl WorkspacePathResolver for FakeFileSystem {
        fn root(&self) -> PathBuf {
            PathBuf::from("/workspace")
        }

        fn resolve(&self, path: &str) -> anyhow::Result<PathBuf> {
            Ok(self.root().join(path))
        }

        fn canonicalize(&self, _path: &Path) -> WorkspaceFuture<'_, PathBuf> {
            let result = self
                .canonical
                .lock()
                .ok()
                .and_then(|mut paths| paths.pop_front())
                .unwrap_or(Err(ErrorKind::Other))
                .map_err(std::io::Error::from);
            Box::pin(async move { result.map_err(anyhow::Error::from) })
        }
    }

    impl WorkspaceWriter for FakeFileSystem {
        fn metadata(&self, _path: &Path) -> WorkspaceFuture<'_, WorkspacePathKind> {
            let result = self.metadata.map_err(std::io::Error::from);
            Box::pin(async move { result.map_err(anyhow::Error::from) })
        }

        fn create_dir_all(&self, path: &Path) -> WorkspaceFuture<'_, ()> {
            let result = self.create_result;
            if result.is_ok()
                && let Ok(mut paths) = self.created.lock()
            {
                paths.push(path.to_path_buf());
            }
            Box::pin(async move {
                result
                    .map_err(std::io::Error::from)
                    .map_err(anyhow::Error::from)
            })
        }

        fn write(&self, path: &Path, contents: &[u8]) -> WorkspaceFuture<'_, ()> {
            let result = self.write_result;
            if result.is_ok()
                && let Ok(mut writes) = self.writes.lock()
            {
                writes.push((path.to_path_buf(), contents.to_vec()));
            }
            Box::pin(async move {
                result
                    .map_err(std::io::Error::from)
                    .map_err(anyhow::Error::from)
            })
        }
    }

    fn tool(file_system: Arc<FakeFileSystem>) -> WriteFile {
        WriteFile {
            root: PathBuf::from("/workspace"),
            path_resolver: file_system.clone(),
            writer: file_system,
            permission: PermissionRequirement::ConfirmationRequired,
        }
    }

    async fn write(
        tool: &WriteFile,
        path: &str,
        content: &str,
    ) -> Result<super::WriteFileOutput, rig::tool::ToolExecutionError> {
        let mut context = ToolContext::default();
        tool.call(
            &mut context,
            WriteFileArgs {
                path: path.to_string(),
                content: content.to_string(),
            },
        )
        .await
    }

    #[test]
    fn schema_requires_only_path_and_content() {
        let file_system = Arc::new(FakeFileSystem::new(Vec::new(), Err(ErrorKind::NotFound)));
        let schema = tool(file_system).parameters();

        assert_eq!(schema["required"], serde_json::json!(["path", "content"]));
        assert_eq!(schema["additionalProperties"], serde_json::json!(false));
        assert_eq!(
            schema["properties"].as_object().map(|value| value.len()),
            Some(2)
        );
        assert_eq!(schema["properties"]["path"]["minLength"], 1);
        assert!(schema["properties"]["content"].get("minLength").is_none());
    }

    #[test]
    fn write_file_requires_confirmation() {
        let file_system = Arc::new(FakeFileSystem::new(Vec::new(), Err(ErrorKind::NotFound)));
        assert_eq!(
            tool(file_system).permission_requirement(),
            PermissionRequirement::ConfirmationRequired
        );
    }

    #[tokio::test]
    async fn new_file_reports_byte_count_and_created_parent() {
        let file_system = Arc::new(FakeFileSystem::new(
            vec![
                Err(ErrorKind::NotFound),
                Err(ErrorKind::NotFound),
                Ok(PathBuf::from("/workspace")),
                Ok(PathBuf::from("/workspace/tasks")),
            ],
            Err(ErrorKind::NotFound),
        ));
        let result = write(&tool(file_system.clone()), "./tasks/example.md", "αβ\n").await;

        assert_eq!(
            result.as_ref().ok().map(|output| output.path.as_str()),
            Some("tasks/example.md")
        );
        assert_eq!(
            result.as_ref().ok().map(|output| output.bytes_written),
            Some(5)
        );
        assert_eq!(
            result.as_ref().ok().map(|output| output.overwritten),
            Some(false)
        );
        assert_eq!(
            result.as_ref().ok().map(|output| output.parent_created),
            Some(true)
        );
        assert_eq!(
            file_system.created.lock().ok().map(|paths| paths.len()),
            Some(1)
        );
        assert_eq!(
            file_system
                .writes
                .lock()
                .ok()
                .and_then(|writes| writes.first().map(|(_, bytes)| bytes.clone())),
            Some("αβ\n".as_bytes().to_vec())
        );
    }

    #[tokio::test]
    async fn existing_regular_file_reports_overwrite_without_parent_creation() {
        let file_system = Arc::new(FakeFileSystem::new(
            vec![
                Ok(PathBuf::from("/workspace/existing.md")),
                Ok(PathBuf::from("/workspace")),
            ],
            Ok(WorkspacePathKind::File),
        ));
        let result = write(&tool(file_system.clone()), "existing.md", "new").await;

        assert_eq!(
            result.as_ref().ok().map(|output| output.overwritten),
            Some(true)
        );
        assert_eq!(
            result.as_ref().ok().map(|output| output.parent_created),
            Some(false)
        );
        assert_eq!(
            file_system.created.lock().ok().map(|paths| paths.len()),
            Some(0)
        );
    }

    #[tokio::test]
    async fn directory_target_is_rejected_before_writing() {
        let file_system = Arc::new(FakeFileSystem::new(
            vec![Ok(PathBuf::from("/workspace/src"))],
            Ok(WorkspacePathKind::Directory),
        ));
        let result = write(&tool(file_system.clone()), "src", "content").await;

        assert_eq!(
            result.err().as_ref().and_then(|error| error.code()),
            Some(error_codes::INVALID_ARGUMENT)
        );
        assert_eq!(
            file_system.writes.lock().ok().map(|writes| writes.len()),
            Some(0)
        );
    }

    #[tokio::test]
    async fn canonical_parent_outside_workspace_is_rejected_before_creation() {
        let file_system = Arc::new(FakeFileSystem::new(
            vec![Err(ErrorKind::NotFound), Ok(PathBuf::from("/outside"))],
            Err(ErrorKind::NotFound),
        ));
        let result = write(&tool(file_system.clone()), "src/main.rs", "content").await;

        assert_eq!(
            result.err().as_ref().and_then(|error| error.code()),
            Some(error_codes::PATH_OUTSIDE_WORKSPACE)
        );
        assert_eq!(
            file_system.created.lock().ok().map(|paths| paths.len()),
            Some(0)
        );
    }

    #[tokio::test]
    async fn permission_failure_has_structured_error_without_success_output() {
        let file_system = Arc::new(
            FakeFileSystem::new(
                vec![
                    Err(ErrorKind::NotFound),
                    Err(ErrorKind::NotFound),
                    Ok(PathBuf::from("/workspace")),
                    Ok(PathBuf::from("/workspace/src")),
                ],
                Err(ErrorKind::NotFound),
            )
            .with_write_error(ErrorKind::PermissionDenied),
        );
        let result = write(&tool(file_system), "src/main.rs", "content").await;

        assert_eq!(
            result.err().as_ref().and_then(|error| error.code()),
            Some(error_codes::PERMISSION_DENIED)
        );
    }

    #[test]
    fn path_policy_rejects_traversal_and_normalizes_dot() {
        assert_eq!(
            validate_path("a/../file.md").ok(),
            Some(PathBuf::from("file.md"))
        );
        assert_eq!(
            validate_path("../file.md")
                .err()
                .as_ref()
                .and_then(|error| error.code()),
            Some(error_codes::PATH_OUTSIDE_WORKSPACE)
        );
        assert_eq!(
            validate_path(".")
                .err()
                .as_ref()
                .and_then(|error| error.code()),
            Some(error_codes::INVALID_ARGUMENT)
        );
    }

    #[test]
    fn path_policy_rejects_traversal_and_normalizes_current_directory() {
        assert_eq!(
            validate_path("a/../file.md").ok(),
            Some(PathBuf::from("file.md"))
        );
        assert_eq!(
            validate_path("../file.md")
                .err()
                .as_ref()
                .and_then(|error| error.code()),
            Some(error_codes::PATH_OUTSIDE_WORKSPACE)
        );
        assert_eq!(
            validate_path(".")
                .err()
                .as_ref()
                .and_then(|error| error.code()),
            Some(error_codes::INVALID_ARGUMENT)
        );
    }

    #[test]
    fn oversized_content_is_rejected_before_filesystem_access() {
        let error = validate_arguments("file.md", &"x".repeat(MAX_CONTENT_BYTES + 1));

        assert_eq!(
            error.err().as_ref().and_then(|value| value.code()),
            Some(error_codes::INVALID_ARGUMENT)
        );
    }

    #[test]
    fn display_path_uses_workspace_relative_forward_slashes() {
        assert_eq!(display_path(Path::new("src/main.rs")), "src/main.rs");
    }

    #[test]
    fn successful_output_serializes_the_complete_write_contract() {
        let output = WriteFileOutput {
            action: Action::Written,
            ok: true,
            path: "tasks/example.md".to_string(),
            bytes_written: 5,
            overwritten: false,
            parent_created: true,
        };
        let value = serde_json::to_value(output).ok();

        assert_eq!(
            value,
            Some(serde_json::json!({
                "action": "written",
                "ok": true,
                "path": "tasks/example.md",
                "bytes_written": 5,
                "overwritten": false,
                "parent_created": true,
            }))
        );
    }

    #[test]
    fn parent_creation_error_uses_parent_error_code() {
        let error = parent_error("src/main.rs", std::io::Error::from(ErrorKind::Other));

        assert_eq!(error.code(), Some(error_codes::PARENT_ERROR));
    }
}
