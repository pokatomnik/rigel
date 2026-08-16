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
    tools::contracts::{CollectionEnvelope, error_codes},
};

const DEFAULT_EXCLUDES: &str = include_str!("excludes.txt");
const CONTEXT_LINES: usize = 2;
const MAX_RESULTS: usize = 200;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SearchTextArgs {
    query: String,
    path: String,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct TextMatch {
    path: String,
    line: usize,
    column: usize,
    text: String,
    before: Vec<ContextLine>,
    after: Vec<ContextLine>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ContextLine {
    line: usize,
    text: String,
}

pub(crate) type SearchTextOutput = CollectionEnvelope<TextMatch>;

struct SearchDirectory {
    path: PathBuf,
    current_directory_relative: PathBuf,
}

struct PathExclusions {
    patterns: Vec<Pattern>,
}

struct MatchBatch {
    items: Vec<TextMatch>,
    has_more: bool,
}

enum TextContent {
    Text(String),
    Binary,
}

pub(crate) struct SearchText {
    root: PathBuf,
    permission: PermissionRequirement,
}

impl SearchText {
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
                    "Cannot access the current directory \"{}\": {error}",
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

    async fn resolve_path(&self, path: &str) -> Result<(PathBuf, PathBuf), ToolExecutionError> {
        let relative = normalize_relative_path(path)?;
        let resolved = fs::canonicalize(self.root.join(&relative))
            .await
            .map_err(|error| resolve_path_error(path, error))?;
        if !resolved.starts_with(&self.root) {
            return Err(outside_current_directory_error(path));
        }
        Ok((resolved, relative))
    }

    async fn search_directory(
        &self,
        directory: PathBuf,
        relative: PathBuf,
        query: &str,
        exclusions: &PathExclusions,
    ) -> Result<SearchTextOutput, ToolExecutionError> {
        let mut directories = VecDeque::from([SearchDirectory {
            path: directory,
            current_directory_relative: relative,
        }]);
        let mut items = Vec::new();

        while let Some(directory) = directories.pop_front() {
            let directory_display = path_for_output(&directory.current_directory_relative);
            let mut entries = read_entries(&directory.path, &directory_display).await?;
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                let relative = directory.current_directory_relative.join(entry.file_name());
                let display_path = path_for_output(&relative);
                let file_type = entry.file_type().await.map_err(|error| {
                    search_io_error(&display_path, "inspecting a directory entry", error)
                })?;
                if file_type.is_symlink() {
                    continue;
                }
                if file_type.is_dir() {
                    if exclusions.matches(&relative, true) {
                        continue;
                    }
                    let resolved = fs::canonicalize(entry.path()).await.map_err(|error| {
                        search_io_error(&display_path, "resolving a directory", error)
                    })?;
                    if !resolved.starts_with(&self.root) {
                        return Err(outside_current_directory_error(&display_path));
                    }
                    directories.push_back(SearchDirectory {
                        path: resolved,
                        current_directory_relative: relative,
                    });
                    continue;
                }
                if !file_type.is_file() || exclusions.matches(&relative, false) {
                    continue;
                }
                let remaining = MAX_RESULTS.saturating_sub(items.len());
                let batch =
                    search_file(&entry.path(), &display_path, query, remaining, false).await?;
                items.extend(batch.items);
                if batch.has_more {
                    return Ok(CollectionEnvelope::new(items, true));
                }
            }
        }
        Ok(finish_output(items, false))
    }
}

impl ToolPermissionMetadata for SearchText {
    fn permission_requirement(&self) -> PermissionRequirement {
        self.permission
    }
}

impl Tool for SearchText {
    const NAME: &'static str = "search_text";
    type Args = SearchTextArgs;
    type Output = SearchTextOutput;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Find exact, case-sensitive text in a file or recursively inside a directory.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Text to find exactly, including letter case."
                },
                "path": {
                    "type": "string",
                    "minLength": 1,
                    "description": "File or directory path relative to the current directory."
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
        validate_query(&args.query)?;
        let exclusions = PathExclusions::new()?;
        let (resolved, relative) = self.resolve_path(&args.path).await?;
        let metadata = fs::metadata(&resolved)
            .await
            .map_err(|error| search_io_error(&args.path, "inspecting the search path", error))?;
        if exclusions.matches(&relative, metadata.is_dir()) {
            return Ok(CollectionEnvelope::new(Vec::new(), false));
        }
        if metadata.is_file() {
            let display_path = path_for_output(&relative);
            let batch =
                search_file(&resolved, &display_path, &args.query, MAX_RESULTS, true).await?;
            return Ok(finish_output(batch.items, batch.has_more));
        }
        if metadata.is_dir() {
            return self
                .search_directory(resolved, relative, &args.query, &exclusions)
                .await;
        }
        Err(coded_error(
            ToolErrorKind::InvalidArgs,
            error_codes::INVALID_PATH_TYPE,
            "Search path is neither a regular file nor a directory.",
        ))
    }
}

impl PathExclusions {
    fn new() -> Result<Self, ToolExecutionError> {
        let patterns = DEFAULT_EXCLUDES
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(|line| {
                Pattern::new(line).map_err(|error| {
                    coded_error(
                        ToolErrorKind::Other,
                        error_codes::IO_ERROR,
                        format!("Cannot compile built-in exclusion \"{line}\": {error}"),
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

fn validate_query(query: &str) -> Result<(), ToolExecutionError> {
    if query.is_empty() {
        return Err(coded_error(
            ToolErrorKind::InvalidArgs,
            error_codes::INVALID_ARGUMENT,
            "Search query must not be empty.",
        ));
    }
    Ok(())
}

async fn search_file(
    path: &Path,
    display_path: &str,
    query: &str,
    limit: usize,
    binary_is_error: bool,
) -> Result<MatchBatch, ToolExecutionError> {
    let bytes = fs::read(path)
        .await
        .map_err(|error| search_io_error(display_path, "reading a file", error))?;
    let content = match decode_text_content(bytes) {
        TextContent::Text(content) => content,
        TextContent::Binary if binary_is_error => {
            return Err(coded_error(
                ToolErrorKind::InvalidArgs,
                error_codes::BINARY_FILE,
                format!("Cannot search \"{display_path}\": the file is binary."),
            ));
        }
        TextContent::Binary => {
            return Ok(MatchBatch {
                items: Vec::new(),
                has_more: false,
            });
        }
    };
    Ok(find_matches(&content, display_path, query, limit))
}

fn find_matches(content: &str, path: &str, query: &str, limit: usize) -> MatchBatch {
    let lines = content.split('\n').collect::<Vec<_>>();
    let mut items = Vec::new();
    for (start, _) in content.match_indices(query) {
        if items.len() == limit {
            return MatchBatch {
                items,
                has_more: true,
            };
        }
        items.push(match_at(content, &lines, path, start));
    }
    MatchBatch {
        items,
        has_more: false,
    }
}

fn match_at(content: &str, lines: &[&str], path: &str, start: usize) -> TextMatch {
    let line_index = content[..start]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count();
    let line_start = content[..start].rfind('\n').map_or(0, |index| index + 1);
    let before_start = line_index.saturating_sub(CONTEXT_LINES);
    let after_end = (line_index + 1 + CONTEXT_LINES).min(lines.len());
    TextMatch {
        path: path.to_string(),
        line: line_index + 1,
        column: content[line_start..start].chars().count() + 1,
        text: lines[line_index].to_string(),
        before: (before_start..line_index)
            .map(|index| ContextLine {
                line: index + 1,
                text: lines[index].to_string(),
            })
            .collect(),
        after: (line_index + 1..after_end)
            .map(|index| ContextLine {
                line: index + 1,
                text: lines[index].to_string(),
            })
            .collect(),
    }
}

async fn read_entries(
    directory: &Path,
    display_path: &str,
) -> Result<Vec<fs::DirEntry>, ToolExecutionError> {
    let mut reader = fs::read_dir(directory)
        .await
        .map_err(|error| search_io_error(display_path, "reading a directory", error))?;
    let mut entries = Vec::new();
    while let Some(entry) = reader
        .next_entry()
        .await
        .map_err(|error| search_io_error(display_path, "reading a directory entry", error))?
    {
        entries.push(entry);
    }
    Ok(entries)
}

fn decode_text_content(bytes: Vec<u8>) -> TextContent {
    if bytes.contains(&0) {
        return TextContent::Binary;
    }
    match String::from_utf8(bytes) {
        Ok(content) => TextContent::Text(content),
        Err(_) => TextContent::Binary,
    }
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

fn resolve_path_error(path: &str, error: std::io::Error) -> ToolExecutionError {
    if error.kind() == std::io::ErrorKind::NotFound {
        return coded_error(
            ToolErrorKind::NotFound,
            error_codes::PATH_NOT_FOUND,
            format!("Cannot search \"{path}\": path does not exist."),
        )
        .with_source(error);
    }
    search_io_error(path, "resolving the search path", error)
}

fn search_io_error(path: &str, operation: &str, error: std::io::Error) -> ToolExecutionError {
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

fn outside_current_directory_error(path: &str) -> ToolExecutionError {
    ToolExecutionError::refused(format!(
        "Cannot search \"{path}\": path is outside the current directory."
    ))
    .with_code(error_codes::PATH_OUTSIDE_CURRENT_DIRECTORY)
}

fn coded_error(
    kind: ToolErrorKind,
    code: &'static str,
    message: impl Into<String>,
) -> ToolExecutionError {
    ToolExecutionError::new(kind, message).with_code(code)
}

fn finish_output(mut items: Vec<TextMatch>, truncated: bool) -> SearchTextOutput {
    items.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.line.cmp(&right.line))
            .then(left.column.cmp(&right.column))
    });
    CollectionEnvelope::new(items, truncated)
}

fn path_for_output(path: &Path) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_contains_only_query_and_path() {
        assert!(
            serde_json::from_value::<SearchTextArgs>(serde_json::json!({
                "query": "ToolRecoveryHook",
                "path": "."
            }))
            .is_ok()
        );
        assert!(
            serde_json::from_value::<SearchTextArgs>(serde_json::json!({
                "query": "x",
                "path": ".",
                "mode": "regex"
            }))
            .is_err()
        );
    }

    #[test]
    fn literal_matching_is_case_sensitive_and_includes_context() {
        let batch = find_matches("before\nAppState\nafter\n", "src/main.rs", "App", 10);

        assert!(!batch.has_more);
        assert_eq!(batch.items.len(), 1);
        assert_eq!(batch.items[0].line, 2);
        assert_eq!(batch.items[0].column, 1);
        assert_eq!(batch.items[0].before[0].text, "before");
        assert_eq!(batch.items[0].after[0].text, "after");
        assert!(find_matches("appstate", "file", "App", 10).items.is_empty());
    }

    #[test]
    fn result_limit_sets_truncated_without_returning_extra_matches() {
        let batch = find_matches("x x x", "file", "x", 2);

        assert_eq!(batch.items.len(), 2);
        assert!(batch.has_more);
    }

    #[test]
    fn binary_detection_rejects_nul_and_invalid_utf8() {
        assert!(matches!(
            decode_text_content(b"text\0binary".to_vec()),
            TextContent::Binary
        ));
        assert!(matches!(
            decode_text_content(vec![0xff]),
            TextContent::Binary
        ));
        assert!(matches!(
            decode_text_content(b"valid UTF-8".to_vec()),
            TextContent::Text(_)
        ));
    }

    #[test]
    fn built_in_exclusions_are_fixed() {
        let exclusions = PathExclusions::new();

        assert!(exclusions.is_ok());
        if let Ok(exclusions) = exclusions {
            assert!(exclusions.matches(Path::new("target"), true));
            assert!(exclusions.matches(Path::new(".git/config"), false));
            assert!(!exclusions.matches(Path::new("src/main.rs"), false));
        }
    }

    #[test]
    fn context_is_bounded_to_server_default() {
        let batch = find_matches("0\n1\n2\n3\n4\n5\n", "file", "3", 10);

        assert_eq!(batch.items[0].before.len(), CONTEXT_LINES);
        assert_eq!(batch.items[0].after.len(), CONTEXT_LINES);
    }
}
