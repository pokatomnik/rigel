use std::{
    collections::VecDeque,
    env,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use glob::{MatchOptions, Pattern};
use ignore::{
    Match,
    gitignore::{Gitignore, GitignoreBuilder},
};
use rig::tool::{Tool, ToolContext, ToolErrorKind, ToolExecutionError, ToolOutput};
use serde::{Deserialize, Serialize};
use tokio::fs;

const DEFAULT_MAX_RESULTS: usize = 100;
const MAX_PATTERNS: usize = 20;
const MAX_RESULTS: usize = 1000;
const IGNORE_FILE_NAMES: [&str; 2] = [".gitignore", ".ignore"];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FindPathsArgs {
    path: String,
    patterns: Vec<String>,
    exclude: Option<Vec<String>>,
    #[serde(default)]
    r#type: PathTypeFilter,
    #[serde(default)]
    include_hidden: bool,
    #[serde(default = "default_respect_ignore_files")]
    respect_ignore_files: bool,
    #[serde(default = "default_max_results")]
    max_results: usize,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum PathTypeFilter {
    #[default]
    File,
    Directory,
    Symlink,
    Any,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum FoundPathType {
    File,
    Directory,
    Symlink,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct FoundPath {
    path: String,
    r#type: FoundPathType,
}

#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct FindPathsOutput {
    paths: Vec<FoundPath>,
    matched: usize,
    truncated: bool,
}

#[derive(Debug)]
struct GlobPatterns {
    patterns: Vec<Pattern>,
}

struct SearchFilters {
    patterns: GlobPatterns,
    exclude: GlobPatterns,
    path_type: PathTypeFilter,
    include_hidden: bool,
}

struct SearchDirectory {
    physical_path: PathBuf,
    logical_path: PathBuf,
    search_relative: PathBuf,
    workspace_relative: PathBuf,
    ignore_stack: IgnoreStack,
}

#[derive(Clone, Default)]
struct IgnoreStack {
    current: Option<Arc<IgnoreLayer>>,
}

struct IgnoreLayer {
    parent: IgnoreStack,
    gitignore: Gitignore,
    ignore: Gitignore,
}

pub(crate) struct FindPaths {
    root: PathBuf,
}

impl FindPaths {
    pub(crate) async fn new() -> Result<Self, ToolExecutionError> {
        let current_dir = env::current_dir().map_err(|error| {
            coded_error(
                ToolErrorKind::Other,
                "IO_ERROR",
                format!("Cannot determine the workspace root: {error}"),
                None,
            )
            .with_source(error)
        })?;
        let root = fs::canonicalize(&current_dir).await.map_err(|error| {
            coded_error(
                ToolErrorKind::Other,
                "IO_ERROR",
                format!(
                    "Cannot access the workspace root \"{}\": {error}",
                    current_dir.display()
                ),
                None,
            )
            .with_source(error)
        })?;

        Ok(Self { root })
    }

    async fn resolve_directory(
        &self,
        path: &str,
    ) -> Result<(PathBuf, PathBuf), ToolExecutionError> {
        let relative = normalize_relative_path(path)?;
        let mut logical = self.root.clone();
        let mut physical = self.root.clone();

        for component in relative.components() {
            let Component::Normal(component) = component else {
                continue;
            };
            logical.push(component);
            physical = fs::canonicalize(&logical)
                .await
                .map_err(|error| resolve_path_error(path, error))?;

            if !physical.starts_with(&self.root) {
                return Err(outside_workspace_error(path));
            }
        }

        let metadata = fs::metadata(&physical)
            .await
            .map_err(|error| find_io_error(path, "inspecting the search root", error))?;

        if !metadata.is_dir() {
            return Err(coded_error(
                ToolErrorKind::InvalidArgs,
                "NOT_A_DIRECTORY",
                "find_paths requires a directory path.",
                Some(serde_json::json!({ "path": path })),
            ));
        }

        Ok((physical, relative))
    }

    async fn search(
        &self,
        physical_root: PathBuf,
        workspace_relative_root: PathBuf,
        filters: &SearchFilters,
        respect_ignore_files: bool,
        max_results: usize,
    ) -> Result<FindPathsOutput, ToolExecutionError> {
        if is_system_git_path(&workspace_relative_root)
            || (!filters.include_hidden && has_hidden_component(&workspace_relative_root))
        {
            return Ok(FindPathsOutput::default());
        }

        let mut initial_ignore_stack = IgnoreStack::default();

        if respect_ignore_files {
            initial_ignore_stack = self
                .load_ancestor_ignore_files(&workspace_relative_root)
                .await?;

            if !workspace_relative_root.as_os_str().is_empty()
                && initial_ignore_stack.is_ignored(&self.root.join(&workspace_relative_root), true)
            {
                return Ok(FindPathsOutput::default());
            }
        }

        let mut directories = VecDeque::from([SearchDirectory {
            physical_path: physical_root,
            logical_path: self.root.join(&workspace_relative_root),
            search_relative: PathBuf::new(),
            workspace_relative: workspace_relative_root,
            ignore_stack: initial_ignore_stack,
        }]);
        let mut paths = Vec::new();

        'search: while let Some(directory) = directories.pop_front() {
            let ignore_stack = if respect_ignore_files {
                load_directory_ignore_files(
                    &self.root,
                    &directory.physical_path,
                    &directory.logical_path,
                    directory.ignore_stack,
                )
                .await?
            } else {
                directory.ignore_stack
            };
            let directory_display = path_for_output(&directory.workspace_relative);
            let mut read_dir = fs::read_dir(&directory.physical_path)
                .await
                .map_err(|error| find_io_error(&directory_display, "reading a directory", error))?;
            let mut entries = Vec::new();

            while let Some(entry) = read_dir.next_entry().await.map_err(|error| {
                find_io_error(&directory_display, "reading a directory entry", error)
            })? {
                entries.push(entry);
            }

            entries.sort_by_key(|entry| entry.file_name());

            for entry in entries {
                let file_name = entry.file_name();
                let search_relative = directory.search_relative.join(&file_name);
                let workspace_relative = directory.workspace_relative.join(&file_name);
                let logical_path = directory.logical_path.join(&file_name);
                let search_glob_path = path_for_glob(&search_relative);
                let display_path = path_for_output(&workspace_relative);
                let file_type = entry.file_type().await.map_err(|error| {
                    find_io_error(&display_path, "inspecting a directory entry", error)
                })?;
                let found_type = if file_type.is_symlink() {
                    Some(FoundPathType::Symlink)
                } else if file_type.is_dir() {
                    Some(FoundPathType::Directory)
                } else if file_type.is_file() {
                    Some(FoundPathType::File)
                } else {
                    None
                };

                if is_system_git_path(&workspace_relative)
                    || (!filters.include_hidden && has_hidden_component(&workspace_relative))
                {
                    continue;
                }

                let is_directory = found_type == Some(FoundPathType::Directory);

                if ignore_stack.is_ignored(&logical_path, is_directory)
                    || filters
                        .exclude
                        .matches_path_or_directory(&search_glob_path, is_directory)
                {
                    continue;
                }

                let physical_directory_path = if is_directory {
                    let physical_path = fs::canonicalize(entry.path()).await.map_err(|error| {
                        find_io_error(&display_path, "resolving a directory", error)
                    })?;

                    if !physical_path.starts_with(&self.root) {
                        return Err(outside_workspace_error(&display_path));
                    }

                    Some(physical_path)
                } else {
                    None
                };

                if let Some(found_type) = found_type
                    && filters.path_type.matches(found_type)
                    && filters.patterns.matches(&search_glob_path)
                {
                    paths.push(FoundPath {
                        path: display_path,
                        r#type: found_type,
                    });

                    if paths.len() > max_results {
                        break 'search;
                    }
                }

                if is_directory {
                    directories.push_back(SearchDirectory {
                        physical_path: physical_directory_path
                            .expect("directory path must be resolved before traversal"),
                        logical_path,
                        search_relative,
                        workspace_relative,
                        ignore_stack: ignore_stack.clone(),
                    });
                }
            }
        }

        Ok(finish_output(paths, max_results))
    }

    async fn load_ancestor_ignore_files(
        &self,
        search_root: &Path,
    ) -> Result<IgnoreStack, ToolExecutionError> {
        if search_root.as_os_str().is_empty() {
            return Ok(IgnoreStack::default());
        }

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
                return Err(outside_workspace_error(&path_for_output(&parent_relative)));
            }

            stack = load_directory_ignore_files(&self.root, &physical_path, &logical_path, stack)
                .await?;
        }

        Ok(stack)
    }
}

impl Tool for FindPaths {
    const NAME: &'static str = "find_paths";
    type Args = FindPathsArgs;
    type Output = FindPathsOutput;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Find paths by glob patterns relative to path. Search is not recursive by default. To search all nested directories, prefix every extension pattern with '**/' (for example, '**/*.rs', '**/*.py', and '**/*.ts'); '*.rs' matches only files directly in path. Set include_hidden for hidden files.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Workspace-relative directory in which to search; '.' means the workspace root"
                },
                "patterns": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 20,
                    "items": {
                        "type": "string",
                        "minLength": 1
                    },
                    "description": "OR glob patterns relative to path: * does not cross '/', ** does, ? matches one character, and [abc] matches one listed character"
                },
                "exclude": {
                    "type": "array",
                    "maxItems": 20,
                    "items": {
                        "type": "string",
                        "minLength": 1
                    },
                    "description": "Glob patterns to exclude, relative to path"
                },
                "type": {
                    "type": "string",
                    "enum": [
                        "file",
                        "directory",
                        "symlink",
                        "any"
                    ],
                    "default": "file"
                },
                "include_hidden": {
                    "type": "boolean",
                    "default": false,
                    "description": "Include paths with a component beginning with '.'; .git is always excluded"
                },
                "respect_ignore_files": {
                    "type": "boolean",
                    "default": true,
                    "description": "Respect nested .gitignore and .ignore files"
                },
                "max_results": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 1000,
                    "default": 100
                }
            },
            "required": [
                "path",
                "patterns"
            ],
            "additionalProperties": false
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        validate_args(&args)?;
        let filters = SearchFilters::new(
            &args.patterns,
            args.exclude.as_deref().unwrap_or(&[]),
            args.r#type,
            args.include_hidden,
        )?;
        let (physical_root, workspace_relative_root) = self.resolve_directory(&args.path).await?;

        self.search(
            physical_root,
            workspace_relative_root,
            &filters,
            args.respect_ignore_files,
            args.max_results,
        )
        .await
    }
}

impl PathTypeFilter {
    fn matches(self, found: FoundPathType) -> bool {
        matches!(
            (self, found),
            (Self::Any, _)
                | (Self::File, FoundPathType::File)
                | (Self::Directory, FoundPathType::Directory)
                | (Self::Symlink, FoundPathType::Symlink)
        )
    }
}

impl SearchFilters {
    fn new(
        patterns: &[String],
        exclude: &[String],
        path_type: PathTypeFilter,
        include_hidden: bool,
    ) -> Result<Self, ToolExecutionError> {
        Ok(Self {
            patterns: GlobPatterns::compile(patterns, "patterns")?,
            exclude: GlobPatterns::compile(exclude, "exclude")?,
            path_type,
            include_hidden,
        })
    }
}

impl GlobPatterns {
    fn compile(patterns: &[String], argument: &str) -> Result<Self, ToolExecutionError> {
        let mut compiled = Vec::with_capacity(patterns.len());

        for pattern in patterns {
            if pattern.is_empty() {
                return Err(coded_error(
                    ToolErrorKind::InvalidArgs,
                    "INVALID_GLOB",
                    format!("{argument} glob patterns must not be empty."),
                    Some(serde_json::json!({ "pattern": pattern })),
                ));
            }

            compiled.push(Pattern::new(pattern).map_err(|error| {
                coded_error(
                    ToolErrorKind::InvalidArgs,
                    "INVALID_GLOB",
                    format!("Invalid {argument} glob \"{pattern}\": {error}"),
                    Some(serde_json::json!({ "pattern": pattern })),
                )
            })?);
        }

        Ok(Self { patterns: compiled })
    }

    fn matches(&self, path: &str) -> bool {
        let options = glob_options();
        self.patterns
            .iter()
            .any(|pattern| pattern.matches_with(path, options))
    }

    fn matches_path_or_directory(&self, path: &str, is_directory: bool) -> bool {
        if self.matches(path) {
            return true;
        }

        is_directory
            && self.matches(&format!(
                "{}/__rigel_descendant__",
                path.trim_end_matches('/')
            ))
    }
}

impl IgnoreStack {
    fn push(self, gitignore: Gitignore, ignore: Gitignore) -> Self {
        if gitignore.is_empty() && ignore.is_empty() {
            return self;
        }

        Self {
            current: Some(Arc::new(IgnoreLayer {
                parent: self,
                gitignore,
                ignore,
            })),
        }
    }

    fn is_ignored(&self, path: &Path, is_directory: bool) -> bool {
        if let Some(ignored) = self.match_layers(path, is_directory, |layer| &layer.ignore) {
            return ignored;
        }

        self.match_layers(path, is_directory, |layer| &layer.gitignore)
            .unwrap_or(false)
    }

    fn match_layers<'a>(
        &'a self,
        path: &Path,
        is_directory: bool,
        matcher: impl Fn(&'a IgnoreLayer) -> &'a Gitignore,
    ) -> Option<bool> {
        let mut current = self.current.as_deref();

        while let Some(layer) = current {
            let matched = matcher(layer).matched(path, is_directory);

            if !matched.is_none() {
                return Some(matches!(matched, Match::Ignore(_)));
            }

            current = layer.parent.current.as_deref();
        }

        None
    }
}

async fn load_directory_ignore_files(
    workspace_root: &Path,
    physical_directory: &Path,
    logical_directory: &Path,
    stack: IgnoreStack,
) -> Result<IgnoreStack, ToolExecutionError> {
    let gitignore = load_ignore_file(
        workspace_root,
        physical_directory,
        logical_directory,
        IGNORE_FILE_NAMES[0],
    )
    .await?;
    let ignore = load_ignore_file(
        workspace_root,
        physical_directory,
        logical_directory,
        IGNORE_FILE_NAMES[1],
    )
    .await?;

    Ok(stack.push(gitignore, ignore))
}

async fn load_ignore_file(
    workspace_root: &Path,
    physical_directory: &Path,
    logical_directory: &Path,
    file_name: &str,
) -> Result<Gitignore, ToolExecutionError> {
    let physical_path = physical_directory.join(file_name);
    let logical_path = logical_directory.join(file_name);
    let resolved_path = match fs::canonicalize(&physical_path).await {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Gitignore::empty());
        }
        Err(error) => {
            return Err(find_io_error(
                &path_for_output(&logical_path),
                "reading an ignore file",
                error,
            ));
        }
    };
    if !resolved_path.starts_with(workspace_root) {
        return Err(outside_workspace_error(&path_for_output(&logical_path)));
    }
    let bytes = fs::read(&resolved_path).await.map_err(|error| {
        find_io_error(
            &path_for_output(&logical_path),
            "reading an ignore file",
            error,
        )
    })?;
    let content = String::from_utf8_lossy(&bytes);

    compile_ignore_content(logical_directory, &logical_path, &content)
}

fn compile_ignore_content(
    directory: &Path,
    source: &Path,
    content: &str,
) -> Result<Gitignore, ToolExecutionError> {
    let mut builder = GitignoreBuilder::new(directory);

    for (index, line) in content.lines().enumerate() {
        let line = if index == 0 {
            line.trim_start_matches('\u{feff}')
        } else {
            line
        };

        builder
            .add_line(Some(source.to_path_buf()), line)
            .map_err(|error| {
                coded_error(
                    ToolErrorKind::InvalidArgs,
                    "INVALID_IGNORE_FILE",
                    format!(
                        "Cannot parse ignore rule at \"{}\":{}: {error}",
                        path_for_output(source),
                        index + 1
                    ),
                    Some(serde_json::json!({
                        "path": path_for_output(source),
                        "line": index + 1
                    })),
                )
            })?;
    }

    builder.build().map_err(|error| {
        coded_error(
            ToolErrorKind::InvalidArgs,
            "INVALID_IGNORE_FILE",
            format!(
                "Cannot compile ignore rules from \"{}\": {error}",
                path_for_output(source)
            ),
            Some(serde_json::json!({ "path": path_for_output(source) })),
        )
    })
}

fn normalize_relative_path(path: &str) -> Result<PathBuf, ToolExecutionError> {
    if path.is_empty() {
        return Err(coded_error(
            ToolErrorKind::InvalidArgs,
            "INVALID_ARGUMENT",
            "Search root must not be empty.",
            Some(serde_json::json!({ "path": path })),
        ));
    }

    let mut normalized = PathBuf::new();

    for component in Path::new(path).components() {
        match component {
            Component::Normal(component) => normalized.push(component),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(outside_workspace_error(path));
            }
        }
    }

    Ok(normalized)
}

fn validate_args(args: &FindPathsArgs) -> Result<(), ToolExecutionError> {
    if args.path.is_empty() {
        return Err(coded_error(
            ToolErrorKind::InvalidArgs,
            "INVALID_ARGUMENT",
            "Search root must not be empty.",
            Some(serde_json::json!({ "path": args.path })),
        ));
    }

    if args.patterns.is_empty() || args.patterns.len() > MAX_PATTERNS {
        return Err(coded_error(
            ToolErrorKind::InvalidArgs,
            "INVALID_ARGUMENT",
            format!("patterns must contain between 1 and {MAX_PATTERNS} glob patterns."),
            Some(serde_json::json!({ "patterns_count": args.patterns.len() })),
        ));
    }

    if args
        .exclude
        .as_ref()
        .is_some_and(|items| items.len() > MAX_PATTERNS)
    {
        return Err(coded_error(
            ToolErrorKind::InvalidArgs,
            "INVALID_ARGUMENT",
            format!("exclude must contain at most {MAX_PATTERNS} glob patterns."),
            Some(serde_json::json!({
                "exclude_count": args.exclude.as_ref().map_or(0, Vec::len)
            })),
        ));
    }

    if !(1..=MAX_RESULTS).contains(&args.max_results) {
        return Err(coded_error(
            ToolErrorKind::InvalidArgs,
            "INVALID_ARGUMENT",
            format!("max_results must be between 1 and {MAX_RESULTS}."),
            Some(serde_json::json!({ "max_results": args.max_results })),
        ));
    }

    Ok(())
}

fn default_respect_ignore_files() -> bool {
    true
}

fn default_max_results() -> usize {
    DEFAULT_MAX_RESULTS
}

fn glob_options() -> MatchOptions {
    MatchOptions {
        case_sensitive: true,
        require_literal_separator: true,
        require_literal_leading_dot: false,
    }
}

fn has_hidden_component(path: &Path) -> bool {
    path.components().any(|component| {
        let Component::Normal(component) = component else {
            return false;
        };
        component.to_string_lossy().starts_with('.')
    })
}

fn is_system_git_path(path: &Path) -> bool {
    path.components().any(|component| {
        matches!(
            component,
            Component::Normal(value) if value.to_string_lossy() == ".git"
        )
    })
}

fn sort_paths(paths: &mut [FoundPath]) {
    paths.sort_by(|left, right| {
        path_depth(&left.path)
            .cmp(&path_depth(&right.path))
            .then_with(|| left.path.cmp(&right.path))
    });
}

fn finish_output(mut paths: Vec<FoundPath>, max_results: usize) -> FindPathsOutput {
    sort_paths(&mut paths);
    let truncated = paths.len() > max_results;

    if truncated {
        paths.truncate(max_results);
    }

    FindPathsOutput {
        matched: paths.len(),
        paths,
        truncated,
    }
}

fn path_depth(path: &str) -> usize {
    path.split('/')
        .filter(|component| !component.is_empty())
        .count()
}

fn path_for_glob(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(component) => Some(component.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn path_for_output(path: &Path) -> String {
    let output = path_for_glob(path);

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
            "PATH_NOT_FOUND",
            "Search root does not exist.",
            Some(serde_json::json!({ "path": path })),
        )
        .with_source(error);
    }

    find_io_error(path, "resolving the search root", error)
}

fn find_io_error(path: &str, operation: &str, error: std::io::Error) -> ToolExecutionError {
    let (kind, code, message) = match error.kind() {
        std::io::ErrorKind::NotFound => (
            ToolErrorKind::NotFound,
            "PATH_NOT_FOUND",
            format!(
                "Cannot continue find_paths for \"{path}\" while {operation}: path no longer exists."
            ),
        ),
        std::io::ErrorKind::PermissionDenied => (
            ToolErrorKind::PermissionDenied,
            "PERMISSION_DENIED",
            format!(
                "Cannot continue find_paths for \"{path}\" while {operation}: permission denied."
            ),
        ),
        _ => (
            ToolErrorKind::Other,
            "IO_ERROR",
            format!("Cannot continue find_paths for \"{path}\" while {operation}: {error}"),
        ),
    };

    coded_error(
        kind,
        code,
        message,
        Some(serde_json::json!({ "path": path, "operation": operation })),
    )
    .with_source(error)
}

fn outside_workspace_error(path: &str) -> ToolExecutionError {
    let message = format!(
        "Search root \"{path}\" is outside the workspace. Use a workspace-relative directory path without '..'."
    );
    with_coded_output(
        ToolExecutionError::refused(message.clone()),
        "PATH_OUTSIDE_WORKSPACE",
        message,
        Some(serde_json::json!({ "path": path })),
    )
}

fn coded_error(
    kind: ToolErrorKind,
    code: &'static str,
    message: impl Into<String>,
    details: Option<serde_json::Value>,
) -> ToolExecutionError {
    let message = message.into();
    with_coded_output(
        ToolExecutionError::new(kind, message.clone()),
        code,
        message,
        details,
    )
}

fn with_coded_output(
    error: ToolExecutionError,
    code: &'static str,
    message: String,
    details: Option<serde_json::Value>,
) -> ToolExecutionError {
    let mut output = serde_json::json!({
        "code": code,
        "message": message,
    });

    if let Some(details) = details {
        output["details"] = details;
    }

    error
        .with_code(code)
        .with_model_output(ToolOutput::json(output))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(value: serde_json::Value) -> FindPathsArgs {
        serde_json::from_value(value).expect("arguments should deserialize")
    }

    #[test]
    fn defaults_match_contract() {
        let args = args(serde_json::json!({
            "path": ".",
            "patterns": ["**/*.rs"]
        }));

        assert_eq!(args.r#type, PathTypeFilter::File);
        assert!(!args.include_hidden);
        assert!(args.respect_ignore_files);
        assert_eq!(args.max_results, 100);
    }

    #[test]
    fn parent_components_are_rejected() {
        let error = normalize_relative_path("src/../secrets").expect_err("path must be rejected");

        assert_eq!(error.code(), Some("PATH_OUTSIDE_WORKSPACE"));
    }

    #[test]
    fn recursive_glob_matches_root_and_nested_paths() {
        let patterns =
            GlobPatterns::compile(&["**/*.rs".to_string()], "patterns").expect("valid glob");

        assert!(patterns.matches("main.rs"));
        assert!(patterns.matches("config/mod.rs"));
        assert!(!patterns.matches("config/mod.toml"));
    }

    #[test]
    fn single_star_does_not_cross_directory_separator() {
        let patterns =
            GlobPatterns::compile(&["*.rs".to_string()], "patterns").expect("valid glob");

        assert!(patterns.matches("main.rs"));
        assert!(!patterns.matches("config/mod.rs"));
    }

    #[test]
    fn exclusion_can_prune_directory_descendants() {
        let patterns =
            GlobPatterns::compile(&["target/**".to_string()], "exclude").expect("valid glob");

        assert!(patterns.matches_path_or_directory("target", true));
        assert!(patterns.matches_path_or_directory("target/debug/app", false));
        assert!(!patterns.matches_path_or_directory("src", true));
    }

    #[test]
    fn hidden_and_git_components_are_detected() {
        assert!(has_hidden_component(Path::new(".github/workflows/ci.yml")));
        assert!(has_hidden_component(Path::new("src/.generated/file.rs")));
        assert!(!has_hidden_component(Path::new("src/main.rs")));
        assert!(is_system_git_path(Path::new(".git/config")));
        assert!(is_system_git_path(Path::new("nested/.git/hooks")));
        assert!(!is_system_git_path(Path::new(".github/workflows")));
    }

    #[test]
    fn ignore_rules_support_negation_without_io() {
        let matcher = compile_ignore_content(
            Path::new("workspace"),
            Path::new("workspace/.gitignore"),
            "*.rs\n!important.rs\n",
        )
        .expect("ignore rules should compile");
        let stack = IgnoreStack::default().push(matcher, Gitignore::empty());

        assert!(stack.is_ignored(Path::new("workspace/generated.rs"), false));
        assert!(!stack.is_ignored(Path::new("workspace/important.rs"), false));
    }

    #[test]
    fn nested_ignore_rules_override_parent_rules() {
        let parent = compile_ignore_content(
            Path::new("workspace"),
            Path::new("workspace/.gitignore"),
            "*.log\n",
        )
        .expect("parent ignore should compile");
        let child = compile_ignore_content(
            Path::new("workspace/logs"),
            Path::new("workspace/logs/.gitignore"),
            "!keep.log\n",
        )
        .expect("child ignore should compile");
        let stack = IgnoreStack::default()
            .push(parent, Gitignore::empty())
            .push(child, Gitignore::empty());

        assert!(stack.is_ignored(Path::new("workspace/logs/drop.log"), false));
        assert!(!stack.is_ignored(Path::new("workspace/logs/keep.log"), false));
    }

    #[test]
    fn ignore_file_has_precedence_over_gitignore() {
        let gitignore = compile_ignore_content(
            Path::new("workspace"),
            Path::new("workspace/.gitignore"),
            "keep.txt\n",
        )
        .expect("gitignore should compile");
        let ignore = compile_ignore_content(
            Path::new("workspace"),
            Path::new("workspace/.ignore"),
            "!keep.txt\n",
        )
        .expect("ignore should compile");
        let stack = IgnoreStack::default().push(gitignore, ignore);

        assert!(!stack.is_ignored(Path::new("workspace/keep.txt"), false));
    }

    #[test]
    fn results_sort_by_depth_then_path() {
        let mut paths = vec![
            FoundPath {
                path: "src/config/mod.rs".to_string(),
                r#type: FoundPathType::File,
            },
            FoundPath {
                path: "src/main.rs".to_string(),
                r#type: FoundPathType::File,
            },
            FoundPath {
                path: "Cargo.toml".to_string(),
                r#type: FoundPathType::File,
            },
            FoundPath {
                path: "crates/core/Cargo.toml".to_string(),
                r#type: FoundPathType::File,
            },
        ];

        sort_paths(&mut paths);

        assert_eq!(
            paths
                .iter()
                .map(|entry| entry.path.as_str())
                .collect::<Vec<_>>(),
            vec![
                "Cargo.toml",
                "src/main.rs",
                "crates/core/Cargo.toml",
                "src/config/mod.rs"
            ]
        );
    }

    #[test]
    fn max_results_truncates_paths_after_deterministic_sorting() {
        let output = finish_output(
            vec![
                FoundPath {
                    path: "src/z.rs".to_string(),
                    r#type: FoundPathType::File,
                },
                FoundPath {
                    path: "Cargo.toml".to_string(),
                    r#type: FoundPathType::File,
                },
                FoundPath {
                    path: "src/a.rs".to_string(),
                    r#type: FoundPathType::File,
                },
            ],
            2,
        );

        assert_eq!(output.matched, 2);
        assert!(output.truncated);
        assert_eq!(
            output
                .paths
                .iter()
                .map(|entry| entry.path.as_str())
                .collect::<Vec<_>>(),
            vec!["Cargo.toml", "src/a.rs"]
        );
    }

    #[test]
    fn invalid_glob_has_machine_readable_code() {
        let error =
            GlobPatterns::compile(&["**/[abc".to_string()], "patterns").expect_err("invalid glob");

        assert_eq!(error.code(), Some("INVALID_GLOB"));
    }

    #[test]
    fn argument_limits_are_validated_without_io() {
        let invalid = args(serde_json::json!({
            "path": ".",
            "patterns": ["**/*"],
            "max_results": 1001
        }));

        let error = validate_args(&invalid).expect_err("limit should be rejected");

        assert_eq!(error.code(), Some("INVALID_ARGUMENT"));
    }
}
