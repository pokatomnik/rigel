use std::path::{Path, PathBuf};

use rig::tool::ToolExecutionError;

use super::search::{SearchContext, SearchFilesOutput};
use crate::tools::{
    error_codes,
    tool_search_files::search_files::SearchFiles,
    utils::{
        errors::file_access_error,
        filesystem::{
            DirectoryEntries, PathKind, canonical_path, invalid_search_path, is_excluded_directory,
            path_kind, read_directory, read_prefix, startup_root,
        },
        path::{MAX_PATH_BYTES, display_workspace_path, validate_relative_path},
        text::decode_text,
    },
};

const MAX_PATTERN_BYTES: usize = 4 * 1024;

const MAX_FILE_BYTES: usize = 1024 * 1024;
const MAX_SCANNED_BYTES: usize = 8 * 1024 * 1024;
const MAX_DIRECTORY_ENTRIES: usize = 2048;
const MAX_DIRECTORIES: usize = 10_000;
const MAX_PENDING_DIRECTORIES: usize = 10_000;

impl SearchFiles {
    /// Creates a read-only search tool rooted at Rigel's startup directory.
    pub(crate) async fn new() -> Result<Self, ToolExecutionError> {
        let root = startup_root("search").await?;
        Ok(Self {
            root,
            permission: crate::shared::tool_permissions::catalog::PermissionRequirement::Automatic,
        })
    }

    /// Searches one file or a bounded directory tree using the supplied literal pattern.
    pub(crate) async fn search_path(
        &self,
        requested_pattern: &str,
        requested_path: &str,
    ) -> Result<SearchFilesOutput, ToolExecutionError> {
        Self::validate_pattern(requested_pattern)?;
        let relative_path = validate_relative_path(requested_path, MAX_PATH_BYTES, "search")?;
        let candidate = self.root.join(relative_path);
        let canonical_path = canonical_path(&self.root, &candidate, requested_path).await?;
        let display_path =
            display_workspace_path(&self.root, &canonical_path, requested_path, "search")?;
        let mut context = SearchContext::new(canonical_path.clone(), requested_pattern);
        self.scan_root(canonical_path, &mut context).await?;
        Ok(context.output(display_path))
    }

    /// Rejects empty, NUL-containing, or oversized literal patterns before I/O.
    pub(crate) fn validate_pattern(pattern: &str) -> Result<(), ToolExecutionError> {
        if pattern.is_empty() || pattern.contains('\0') || pattern.len() > MAX_PATTERN_BYTES {
            return Err(ToolExecutionError::invalid_args(
                "Cannot search files: pattern must be non-empty, contain no NUL characters, and be at most 4096 bytes.",
            )
            .with_code(error_codes::INVALID_ARGUMENT));
        }
        Ok(())
    }

    async fn scan_root(
        &self,
        root: PathBuf,
        context: &mut SearchContext,
    ) -> Result<(), ToolExecutionError> {
        match path_kind(&root)
            .await
            .map_err(|error| file_access_error(root.display(), error, "search"))?
        {
            PathKind::File => self.scan_file(root, context).await,
            PathKind::Directory if is_excluded_directory(&root) => Ok(()),
            PathKind::Directory => self.walk_directories(context).await,
            PathKind::Other | PathKind::Symlink => Err(invalid_search_path(root.display())),
        }
    }

    async fn walk_directories(
        &self,
        context: &mut SearchContext,
    ) -> Result<(), ToolExecutionError> {
        let mut pending = vec![context.root()];
        while let Some(directory) = pending.pop() {
            if context.directories_reached(MAX_DIRECTORIES) {
                context.mark_hard_limit();
                break;
            }
            context.count_directory();
            self.scan_directory(directory, context, &mut pending)
                .await?;
            if context.should_stop() {
                break;
            }
        }
        Ok(())
    }

    async fn scan_directory(
        &self,
        directory: PathBuf,
        context: &mut SearchContext,
        pending: &mut Vec<PathBuf>,
    ) -> Result<(), ToolExecutionError> {
        let entries = Self::directory_entries(&directory, context).await?;
        let truncated = entries.truncated;
        let mut paths = entries.paths;
        paths.sort_by(|left, right| left.to_string_lossy().cmp(&right.to_string_lossy()));
        for path in paths {
            if !self.queue_entry(path, context, pending).await? {
                break;
            }
        }
        if truncated {
            context.mark_hard_limit();
        }
        pending.sort_by(|left, right| right.to_string_lossy().cmp(&left.to_string_lossy()));
        Ok(())
    }

    async fn directory_entries(
        directory: &Path,
        context: &mut SearchContext,
    ) -> Result<DirectoryEntries, ToolExecutionError> {
        match read_directory(directory, MAX_DIRECTORY_ENTRIES).await {
            Ok(entries) => Ok(entries),
            Err(error) if directory == context.root() => {
                Err(file_access_error(directory.display(), error, "search"))
            }
            Err(_) => {
                context.warning(format!(
                    "Skipped unreadable directory \"{}\".",
                    directory.display()
                ));
                Ok(DirectoryEntries {
                    paths: Vec::new(),
                    truncated: false,
                })
            }
        }
    }

    async fn queue_entry(
        &self,
        path: PathBuf,
        context: &mut SearchContext,
        pending: &mut Vec<PathBuf>,
    ) -> Result<bool, ToolExecutionError> {
        if context.should_stop() {
            return Ok(false);
        }
        if let Some(child_directory) = self.scan_entry(path, context).await? {
            if pending.len() >= MAX_PENDING_DIRECTORIES {
                context.mark_hard_limit();
                return Ok(false);
            }
            pending.push(child_directory);
        }
        Ok(true)
    }

    async fn scan_entry(
        &self,
        path: PathBuf,
        context: &mut SearchContext,
    ) -> Result<Option<PathBuf>, ToolExecutionError> {
        let Some(kind) = Self::entry_kind(&path, context).await else {
            return Ok(None);
        };
        if kind == PathKind::Symlink {
            return self.inspect_symlink(path, context).await;
        }
        if kind == PathKind::Directory && is_excluded_directory(&path) {
            return Ok(None);
        }
        let Some(canonical) = self.canonical_entry(&path, context).await? else {
            return Ok(None);
        };
        match kind {
            PathKind::File => {
                if let Err(error) = self.scan_file(canonical, context).await {
                    self.record_file_error(path, error, context)?;
                }
                Ok(None)
            }
            PathKind::Directory => Ok(Some(canonical)),
            PathKind::Other | PathKind::Symlink => Ok(None),
        }
    }

    async fn entry_kind(path: &Path, context: &mut SearchContext) -> Option<PathKind> {
        match path_kind(path).await {
            Ok(kind) => Some(kind),
            Err(_) => {
                context.warning(format!("Skipped inaccessible path \"{}\".", path.display()));
                None
            }
        }
    }

    async fn canonical_entry(
        &self,
        path: &Path,
        context: &mut SearchContext,
    ) -> Result<Option<PathBuf>, ToolExecutionError> {
        match canonical_path(&self.root, path, &path.to_string_lossy()).await {
            Ok(path) => Ok(Some(path)),
            Err(error) if error.code() == Some(error_codes::PATH_OUTSIDE_WORKSPACE) => Err(error),
            Err(_) => {
                context.warning(format!("Skipped inaccessible path \"{}\".", path.display()));
                Ok(None)
            }
        }
    }

    async fn inspect_symlink(
        &self,
        path: PathBuf,
        context: &mut SearchContext,
    ) -> Result<Option<PathBuf>, ToolExecutionError> {
        match canonical_path(&self.root, &path, &path.to_string_lossy()).await {
            Err(error) if error.code() == Some(error_codes::PATH_OUTSIDE_WORKSPACE) => Err(error),
            _ => {
                context.warning(format!("Skipped symlink \"{}\".", path.display()));
                Ok(None)
            }
        }
    }

    async fn scan_file(
        &self,
        path: PathBuf,
        context: &mut SearchContext,
    ) -> Result<(), ToolExecutionError> {
        let remaining = context.remaining_bytes(MAX_SCANNED_BYTES);
        if remaining == 0 {
            context.mark_hard_limit();
            return Ok(());
        }
        let limit = remaining.min(MAX_FILE_BYTES);
        let read = read_prefix(&path, limit)
            .await
            .map_err(|error| file_access_error(path.display(), error, "search"))?;
        context.consume_bytes(read.bytes.len());
        if read.truncated {
            context.mark_scan_limit();
        }
        let Some(text) = decode_text(read.bytes) else {
            context.warning(format!(
                "Skipped binary or non-text file \"{}\".",
                path.display()
            ));
            return Ok(());
        };
        let display = display_workspace_path(&self.root, &path, &path.to_string_lossy(), "search")?;
        context.record_matches(&display, &text);
        Ok(())
    }

    fn record_file_error(
        &self,
        path: PathBuf,
        error: ToolExecutionError,
        context: &mut SearchContext,
    ) -> Result<(), ToolExecutionError> {
        if error.code() == Some(error_codes::PATH_OUTSIDE_WORKSPACE) {
            return Err(error);
        }
        context.warning(format!("Skipped unreadable file \"{}\".", path.display()));
        Ok(())
    }
}
