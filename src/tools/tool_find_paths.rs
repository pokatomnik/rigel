use std::{
    collections::VecDeque,
    env,
    path::{Component, Path, PathBuf},
};

use glob::Pattern;
use rig::tool::{Tool, ToolContext, ToolErrorKind, ToolExecutionError};
use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::{
    shared::tool_permissions::{PermissionRequirement, ToolPermissionMetadata},
    tools::{
        contracts::{CollectionEnvelope, error_codes},
        find_paths_ignore::{IgnoreStack, load_directory_ignore_files},
    },
};

const MAX_RESULTS: usize = 500;
const DEFAULT_EXCLUDES: &str = include_str!("excludes.txt");

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FindPathsArgs {
    query: String,
    path: String,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct FoundPath {
    path: String,
    #[serde(rename = "type")]
    kind: FoundPathKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum FoundPathKind {
    File,
    Directory,
}

pub(crate) type FindPathsOutput = CollectionEnvelope<FoundPath>;

struct NameQuery {
    normalized: String,
}

struct BuiltInExclusions {
    patterns: Vec<Pattern>,
}

struct SearchDirectory {
    physical_path: PathBuf,
    logical_path: PathBuf,
    current_directory_relative: PathBuf,
    ignore_stack: IgnoreStack,
}

struct SearchState {
    directories: VecDeque<SearchDirectory>,
    paths: Vec<FoundPath>,
    query: NameQuery,
    exclusions: BuiltInExclusions,
}

struct ScannedEntry {
    name: String,
    relative: PathBuf,
    logical_path: PathBuf,
    display_path: String,
    kind: FoundPathKind,
    physical_directory: Option<PathBuf>,
}

pub(crate) struct FindPaths {
    root: PathBuf,
    permission: PermissionRequirement,
}

impl FindPaths {
    pub(crate) async fn new() -> Result<Self, ToolExecutionError> {
        let current_dir = env::current_dir().map_err(|error| {
            coded_error(
                ToolErrorKind::Other,
                error_codes::IO_ERROR,
                format!("Cannot determine the current directory: {error}"),
            )
            .with_source(error)
        })?;
        let root = fs::canonicalize(&current_dir).await.map_err(|error| {
            coded_error(
                ToolErrorKind::Other,
                error_codes::IO_ERROR,
                format!(
                    "Cannot access the current directory root \"{}\": {error}",
                    current_dir.display()
                ),
            )
            .with_source(error)
        })?;
        Ok(Self {
            root,
            permission: PermissionRequirement::Automatic,
        })
    }

    async fn resolve_directory(
        &self,
        path: &str,
    ) -> Result<(PathBuf, PathBuf), ToolExecutionError> {
        let relative = normalize_relative_path(path)?;
        let resolved = fs::canonicalize(self.root.join(&relative))
            .await
            .map_err(|error| resolve_path_error(path, error))?;
        if !resolved.starts_with(&self.root) {
            return Err(outside_current_directory_error(path));
        }

        let metadata = fs::metadata(&resolved)
            .await
            .map_err(|error| find_io_error(path, "inspecting the search path", error))?;
        if !metadata.is_dir() {
            return Err(coded_error(
                ToolErrorKind::InvalidArgs,
                error_codes::INVALID_PATH_TYPE,
                format!("Cannot search \"{path}\": path is not a directory."),
            ));
        }

        Ok((resolved, relative))
    }

    async fn search(
        &self,
        physical_root: PathBuf,
        current_directory_relative: PathBuf,
        query: NameQuery,
    ) -> Result<FindPathsOutput, ToolExecutionError> {
        let Some(mut state) = self
            .initialize_search(physical_root, current_directory_relative, query)
            .await?
        else {
            return Ok(CollectionEnvelope::new(Vec::new(), false));
        };

        while let Some(directory) = state.directories.pop_front() {
            self.scan_directory(directory, &mut state).await?;
            if state.paths.len() > MAX_RESULTS {
                break;
            }
        }

        Ok(finish_output(state.paths))
    }

    async fn initialize_search(
        &self,
        physical_root: PathBuf,
        current_directory_relative: PathBuf,
        query: NameQuery,
    ) -> Result<Option<SearchState>, ToolExecutionError> {
        let exclusions = BuiltInExclusions::new()?;
        if exclusions.matches(&current_directory_relative, true) {
            return Ok(None);
        }
        let ignore_stack = self
            .load_ancestor_ignore_files(&current_directory_relative)
            .await?;
        let logical_root = self.root.join(&current_directory_relative);
        if !current_directory_relative.as_os_str().is_empty()
            && ignore_stack.is_ignored(&logical_root, true)
        {
            return Ok(None);
        }
        Ok(Some(SearchState {
            directories: VecDeque::from([SearchDirectory {
                physical_path: physical_root,
                logical_path: logical_root,
                current_directory_relative,
                ignore_stack,
            }]),
            paths: Vec::new(),
            query,
            exclusions,
        }))
    }

    async fn scan_directory(
        &self,
        mut directory: SearchDirectory,
        state: &mut SearchState,
    ) -> Result<(), ToolExecutionError> {
        let ignore_stack = load_directory_ignore_files(
            &self.root,
            &directory.physical_path,
            &directory.logical_path,
            directory.ignore_stack.clone(),
        )
        .await?;
        directory.ignore_stack = ignore_stack;
        let directory_display = path_for_output(&directory.current_directory_relative);
        let mut entries = read_entries(&directory.physical_path, &directory_display).await?;
        entries.sort_by_key(|entry| entry.file_name());

        for entry in entries {
            self.scan_entry(entry, &directory, state).await?;
            if state.paths.len() > MAX_RESULTS {
                break;
            }
        }

        Ok(())
    }

    async fn scan_entry(
        &self,
        entry: fs::DirEntry,
        directory: &SearchDirectory,
        state: &mut SearchState,
    ) -> Result<(), ToolExecutionError> {
        let Some(entry) = self.inspect_entry(entry, directory).await? else {
            return Ok(());
        };
        if state
            .exclusions
            .matches(&entry.relative, entry.kind == FoundPathKind::Directory)
            || directory
                .ignore_stack
                .is_ignored(&entry.logical_path, entry.kind == FoundPathKind::Directory)
        {
            return Ok(());
        }
        if state.query.matches(&entry.name) {
            state.paths.push(FoundPath {
                path: entry.display_path,
                kind: entry.kind,
            });
        }
        if let Some(physical_path) = entry.physical_directory {
            state.directories.push_back(SearchDirectory {
                physical_path,
                logical_path: entry.logical_path,
                current_directory_relative: entry.relative,
                ignore_stack: directory.ignore_stack.clone(),
            });
        }
        Ok(())
    }

    async fn inspect_entry(
        &self,
        entry: fs::DirEntry,
        directory: &SearchDirectory,
    ) -> Result<Option<ScannedEntry>, ToolExecutionError> {
        let file_name = entry.file_name();
        let relative = directory.current_directory_relative.join(&file_name);
        let display_path = path_for_output(&relative);
        let logical_path = directory.logical_path.join(&file_name);
        let file_type = entry
            .file_type()
            .await
            .map_err(|error| find_io_error(&display_path, "inspecting a directory entry", error))?;

        if file_type.is_symlink() {
            return Ok(None);
        }

        let kind = if file_type.is_dir() {
            FoundPathKind::Directory
        } else if file_type.is_file() {
            FoundPathKind::File
        } else {
            return Ok(None);
        };
        let is_directory = kind == FoundPathKind::Directory;
        let physical_directory = if is_directory {
            let physical_path = fs::canonicalize(entry.path())
                .await
                .map_err(|error| find_io_error(&display_path, "resolving a directory", error))?;
            if !physical_path.starts_with(&self.root) {
                return Err(outside_current_directory_error(&display_path));
            }
            Some(physical_path)
        } else {
            None
        };
        Ok(Some(ScannedEntry {
            name: file_name.to_string_lossy().into_owned(),
            relative,
            logical_path,
            display_path,
            kind,
            physical_directory,
        }))
    }

    async fn load_ancestor_ignore_files(
        &self,
        search_root: &Path,
    ) -> Result<IgnoreStack, ToolExecutionError> {
        let components = search_root.components().collect::<Vec<_>>();
        let mut stack =
            load_directory_ignore_files(&self.root, &self.root, &self.root, IgnoreStack::default())
                .await?;
        let mut parent_relative = PathBuf::new();

        for component in components.iter().take(components.len().saturating_sub(1)) {
            let Component::Normal(component) = component else {
                continue;
            };
            parent_relative.push(component);
            let logical_path = self.root.join(&parent_relative);
            let physical_path = fs::canonicalize(&logical_path)
                .await
                .map_err(|error| resolve_path_error(&path_for_output(&parent_relative), error))?;
            if !physical_path.starts_with(&self.root) {
                return Err(outside_current_directory_error(&path_for_output(
                    &parent_relative,
                )));
            }
            stack = load_directory_ignore_files(&self.root, &physical_path, &logical_path, stack)
                .await?;
        }

        Ok(stack)
    }
}

impl ToolPermissionMetadata for FindPaths {
    fn permission_requirement(&self) -> PermissionRequirement {
        self.permission
    }
}

impl Tool for FindPaths {
    const NAME: &'static str = "find_paths";
    type Args = FindPathsArgs;
    type Output = FindPathsOutput;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Find files and directories by case-insensitive name match inside the current directory."
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "minLength": 1,
                    "description": "A file or directory name, or part of a name."
                },
                "path": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Directory path relative to the current directory; '.' means the current directory."
                }
            },
            "required": ["query", "path"],
            "additionalProperties": false
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let query = NameQuery::new(&args.query)?;
        let (physical_root, relative_path) = self.resolve_directory(&args.path).await?;
        self.search(physical_root, relative_path, query).await
    }
}

impl NameQuery {
    fn new(query: &str) -> Result<Self, ToolExecutionError> {
        if query.is_empty() {
            return Err(coded_error(
                ToolErrorKind::InvalidArgs,
                error_codes::INVALID_ARGUMENT,
                "Search query must not be empty.",
            ));
        }
        Ok(Self {
            normalized: query.to_lowercase(),
        })
    }

    fn matches(&self, name: &str) -> bool {
        name.to_lowercase().contains(&self.normalized)
    }
}

impl BuiltInExclusions {
    fn new() -> Result<Self, ToolExecutionError> {
        let patterns = DEFAULT_EXCLUDES
            .lines()
            .map(str::trim)
            .filter(|pattern| !pattern.is_empty() && !pattern.starts_with('#'))
            .map(|pattern| {
                Pattern::new(pattern).map_err(|error| {
                    coded_error(
                        ToolErrorKind::Other,
                        error_codes::IO_ERROR,
                        format!("Cannot compile built-in exclusion \"{pattern}\": {error}"),
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { patterns })
    }

    fn matches(&self, path: &Path, is_directory: bool) -> bool {
        let path = path_for_output(path);
        self.patterns.iter().any(|pattern| {
            pattern.matches(&path)
                || (is_directory && pattern.matches(&format!("{path}/__rigel_descendant__")))
        })
    }
}

async fn read_entries(
    directory: &Path,
    display_path: &str,
) -> Result<Vec<fs::DirEntry>, ToolExecutionError> {
    let mut reader = fs::read_dir(directory)
        .await
        .map_err(|error| find_io_error(display_path, "reading a directory", error))?;
    let mut entries = Vec::new();
    while let Some(entry) = reader
        .next_entry()
        .await
        .map_err(|error| find_io_error(display_path, "reading a directory entry", error))?
    {
        entries.push(entry);
    }
    Ok(entries)
}

fn normalize_relative_path(path: &str) -> Result<PathBuf, ToolExecutionError> {
    if path.is_empty() {
        return Err(coded_error(
            ToolErrorKind::InvalidArgs,
            error_codes::INVALID_ARGUMENT,
            "Search path must not be empty.",
        ));
    }
    let mut normalized = PathBuf::new();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(component) => normalized.push(component),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(outside_current_directory_error(path));
            }
        }
    }
    Ok(normalized)
}

fn finish_output(mut paths: Vec<FoundPath>) -> FindPathsOutput {
    paths.sort_by(|left, right| left.path.cmp(&right.path));
    let truncated = paths.len() > MAX_RESULTS;
    if truncated {
        paths.truncate(MAX_RESULTS);
    }
    CollectionEnvelope::new(paths, truncated)
}

pub(crate) fn path_for_output(path: &Path) -> String {
    let output = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(component) => Some(component.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/");
    if output.is_empty() {
        ".".to_string()
    } else {
        output
    }
}

fn resolve_path_error(path: &str, error: std::io::Error) -> ToolExecutionError {
    if error.kind() == std::io::ErrorKind::NotFound {
        return coded_error(
            ToolErrorKind::NotFound,
            error_codes::PATH_NOT_FOUND,
            format!("Cannot search \"{path}\": path does not exist."),
        )
        .with_source(error);
    }
    find_io_error(path, "resolving the search path", error)
}

pub(crate) fn find_io_error(
    path: &str,
    operation: &str,
    error: std::io::Error,
) -> ToolExecutionError {
    let (kind, code, message) = match error.kind() {
        std::io::ErrorKind::NotFound => (
            ToolErrorKind::NotFound,
            error_codes::PATH_NOT_FOUND,
            format!("Cannot search \"{path}\" while {operation}: path does not exist."),
        ),
        std::io::ErrorKind::PermissionDenied => (
            ToolErrorKind::PermissionDenied,
            error_codes::PERMISSION_DENIED,
            format!("Cannot search \"{path}\" while {operation}: permission denied."),
        ),
        _ => (
            ToolErrorKind::Other,
            error_codes::IO_ERROR,
            format!("Cannot search \"{path}\" while {operation}: {error}"),
        ),
    };
    coded_error(kind, code, message).with_source(error)
}

pub(crate) fn outside_current_directory_error(path: &str) -> ToolExecutionError {
    coded_error(
        ToolErrorKind::PermissionDenied,
        error_codes::PATH_OUTSIDE_CURRENT_DIRECTORY,
        format!(
            "Cannot search \"{path}\": path is outside the current directory. Use a relative path without '..'."
        ),
    )
}

pub(crate) fn coded_error(
    kind: ToolErrorKind,
    code: &'static str,
    message: impl Into<String>,
) -> ToolExecutionError {
    let message = message.into();
    ToolExecutionError::new(kind, message).with_code(code)
}

#[cfg(test)]
mod tests {
    use ignore::gitignore::Gitignore;

    use crate::tools::find_paths_ignore::compile_ignore_content;

    use super::*;

    fn parse_args(value: serde_json::Value) -> Result<FindPathsArgs, serde_json::Error> {
        serde_json::from_value(value)
    }

    #[test]
    fn schema_args_are_only_query_and_path() {
        assert!(
            parse_args(serde_json::json!({
                "query": "chat",
                "path": "."
            }))
            .is_ok()
        );
        assert!(
            parse_args(serde_json::json!({
                "query": "chat",
                "path": ".",
                "type": "file"
            }))
            .is_err()
        );
    }

    #[test]
    fn query_matching_ignores_case_and_matches_name_parts() {
        let query = NameQuery::new("ChAt");

        assert!(query.is_ok());
        if let Ok(query) = query {
            assert!(query.matches("myChat.rs"));
            assert!(!query.matches("main.rs"));
        }
    }

    #[test]
    fn empty_query_is_invalid() {
        let error = NameQuery::new("");

        assert!(error.is_err());
        if let Err(error) = error {
            assert_eq!(error.code(), Some(error_codes::INVALID_ARGUMENT));
        }
    }

    #[test]
    fn relative_path_rejects_absolute_and_parent_paths() {
        assert!(normalize_relative_path("../outside").is_err());
        assert!(normalize_relative_path("/outside").is_err());
        assert_eq!(
            normalize_relative_path("./src").ok(),
            Some(PathBuf::from("src"))
        );
    }

    #[test]
    fn built_in_exclusions_match_directories_and_descendants() {
        let exclusions = BuiltInExclusions::new();

        assert!(exclusions.is_ok());
        if let Ok(exclusions) = exclusions {
            assert!(exclusions.matches(Path::new(".git"), true));
            assert!(exclusions.matches(Path::new("target/debug/app"), false));
            assert!(exclusions.matches(Path::new("node_modules"), true));
            assert!(!exclusions.matches(Path::new("src/main.rs"), false));
        }
    }

    #[test]
    fn result_limit_returns_500_items_and_marks_truncation() {
        let paths = (0..=MAX_RESULTS)
            .map(|index| FoundPath {
                path: format!("src/file-{index}.rs"),
                kind: FoundPathKind::File,
            })
            .collect();
        let output = finish_output(paths);

        assert_eq!(output.count, MAX_RESULTS);
        assert_eq!(output.items.len(), MAX_RESULTS);
        assert!(output.truncated);
    }

    #[test]
    fn results_are_sorted_by_normalized_relative_path() {
        let output = finish_output(vec![
            FoundPath {
                path: "src/z.rs".to_string(),
                kind: FoundPathKind::File,
            },
            FoundPath {
                path: "Cargo.toml".to_string(),
                kind: FoundPathKind::File,
            },
        ]);

        assert_eq!(output.items[0].path, "Cargo.toml");
        assert_eq!(output.items[1].path, "src/z.rs");
    }

    #[test]
    fn ignore_rules_support_negation_without_io() {
        let matcher = compile_ignore_content(
            Path::new("workspace"),
            Path::new("workspace/.gitignore"),
            "*.rs\n!important.rs\n",
        );

        assert!(matcher.is_ok());
        if let Ok(matcher) = matcher {
            let stack = IgnoreStack::default().push(matcher, Gitignore::empty());
            assert!(stack.is_ignored(Path::new("workspace/generated.rs"), false));
            assert!(!stack.is_ignored(Path::new("workspace/important.rs"), false));
        }
    }
}
