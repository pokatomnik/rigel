use std::{
    collections::VecDeque,
    env,
    path::{Component, Path, PathBuf},
};

use glob::{MatchOptions, Pattern};
use regex::{Regex, RegexBuilder};
use rig::tool::{Tool, ToolContext, ToolErrorKind, ToolExecutionError, ToolOutput};
use serde::{Deserialize, Serialize};
use tokio::fs;

const DEFAULT_EXCLUDES: &str = include_str!("excludes.txt");
const DEFAULT_CONTEXT_LINES: usize = 2;
const DEFAULT_MAX_RESULTS: usize = 50;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SearchTextArgs {
    query: String,
    path: String,
    #[serde(default)]
    mode: SearchMode,
    #[serde(default = "default_case_sensitive")]
    case_sensitive: bool,
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
    #[serde(default = "default_context_lines")]
    context_lines: usize,
    #[serde(default = "default_max_results")]
    max_results: usize,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum SearchMode {
    #[default]
    Literal,
    Regex,
}

#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct SearchTextOutput {
    files: Vec<FileMatches>,
    matched_files: usize,
    matched_occurrences: usize,
    truncated: bool,
    skipped_binary_files: usize,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct FileMatches {
    path: String,
    matches: Vec<TextMatch>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct TextMatch {
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

struct SearchFilters {
    include: Option<GlobPatterns>,
    exclude: GlobPatterns,
    default_exclude: GlobPatterns,
}

struct GlobPatterns {
    patterns: Vec<Pattern>,
}

struct SearchDirectory {
    path: PathBuf,
    search_relative: PathBuf,
    current_directory_relative: PathBuf,
}

struct ContentMatches {
    matches: Vec<TextMatch>,
    has_more: bool,
}

enum TextContent {
    Text(String),
    Binary,
}

pub(crate) struct SearchText {
    root: PathBuf,
}

impl SearchText {
    pub(crate) async fn new() -> Result<Self, ToolExecutionError> {
        let current_dir = env::current_dir().map_err(|error| {
            coded_error(
                ToolErrorKind::Other,
                "IO_ERROR",
                format!("Cannot determine the current directory: {error}"),
            )
            .with_source(error)
        })?;
        let root = fs::canonicalize(&current_dir).await.map_err(|error| {
            coded_error(
                ToolErrorKind::Other,
                "IO_ERROR",
                format!(
                    "Cannot access the current directory \"{}\": {error}",
                    current_dir.display()
                ),
            )
            .with_source(error)
        })?;

        Ok(Self { root })
    }

    fn normalize_relative_path(path: &str) -> Result<PathBuf, ToolExecutionError> {
        if path.is_empty() {
            return Err(coded_error(
                ToolErrorKind::InvalidArgs,
                "INVALID_ARGUMENT",
                "Search path must not be empty.",
            ));
        }

        let mut normalized = PathBuf::new();

        for component in Path::new(path).components() {
            match component {
                Component::Normal(component) => normalized.push(component),
                Component::ParentDir if !normalized.pop() => {
                    return Err(outside_current_directory_error(path));
                }
                Component::ParentDir | Component::CurDir => {}
                Component::RootDir | Component::Prefix(_) => {
                    return Err(outside_current_directory_error(path));
                }
            }
        }

        Ok(normalized)
    }

    async fn resolve_path(&self, path: &str) -> Result<(PathBuf, PathBuf), ToolExecutionError> {
        let relative_path = Self::normalize_relative_path(path)?;
        let resolved = fs::canonicalize(self.root.join(&relative_path))
            .await
            .map_err(|error| resolve_path_error(path, error))?;

        if !resolved.starts_with(&self.root) {
            return Err(outside_current_directory_error(path));
        }

        Ok((resolved, relative_path))
    }

    async fn search_directory(
        &self,
        directory: PathBuf,
        current_directory_relative: PathBuf,
        regex: &Regex,
        filters: &SearchFilters,
        context_lines: usize,
        max_results: usize,
    ) -> Result<SearchTextOutput, ToolExecutionError> {
        let mut output = SearchTextOutput::default();

        if filters
            .default_exclude
            .matches_directory(&path_for_glob(&current_directory_relative))
        {
            return Ok(output);
        }

        let mut directories = VecDeque::from([SearchDirectory {
            path: directory,
            search_relative: PathBuf::new(),
            current_directory_relative,
        }]);

        'search: while let Some(directory) = directories.pop_front() {
            let directory_display = path_for_output(&directory.current_directory_relative);
            let mut read_dir = fs::read_dir(&directory.path).await.map_err(|error| {
                search_io_error(&directory_display, "reading a directory", error)
            })?;
            let mut entries = Vec::new();

            while let Some(entry) = read_dir.next_entry().await.map_err(|error| {
                search_io_error(&directory_display, "reading a directory entry", error)
            })? {
                entries.push(entry);
            }

            entries.sort_by_key(|entry| entry.file_name());

            for entry in entries {
                let file_name = entry.file_name();
                let search_relative = directory.search_relative.join(&file_name);
                let current_directory_relative =
                    directory.current_directory_relative.join(&file_name);
                let search_glob_path = path_for_glob(&search_relative);
                let current_directory_glob_path = path_for_glob(&current_directory_relative);
                let display_path = path_for_output(&current_directory_relative);
                let file_type = entry.file_type().await.map_err(|error| {
                    search_io_error(&display_path, "inspecting a directory entry", error)
                })?;

                if file_type.is_dir() {
                    if filters
                        .should_exclude_directory(&search_glob_path, &current_directory_glob_path)
                    {
                        continue;
                    }

                    let resolved = fs::canonicalize(entry.path()).await.map_err(|error| {
                        search_io_error(&display_path, "resolving a directory", error)
                    })?;

                    if !resolved.starts_with(&self.root) {
                        continue;
                    }

                    directories.push_back(SearchDirectory {
                        path: resolved,
                        search_relative,
                        current_directory_relative,
                    });
                } else if file_type.is_file() {
                    if !filters.should_search_file(&search_glob_path, &current_directory_glob_path)
                    {
                        continue;
                    }

                    if self
                        .search_one_file(
                            &entry.path(),
                            &current_directory_relative,
                            regex,
                            context_lines,
                            max_results,
                            &mut output,
                            false,
                        )
                        .await?
                    {
                        break 'search;
                    }
                } else if file_type.is_symlink() {
                    let resolved = match fs::canonicalize(entry.path()).await {
                        Ok(resolved) if resolved.starts_with(&self.root) => resolved,
                        Ok(_) => continue,
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                        Err(error) => {
                            return Err(search_io_error(
                                &display_path,
                                "resolving a symbolic link",
                                error,
                            ));
                        }
                    };
                    let metadata = fs::metadata(&resolved).await.map_err(|error| {
                        search_io_error(&display_path, "inspecting a symbolic link target", error)
                    })?;

                    if metadata.is_file()
                        && filters
                            .should_search_file(&search_glob_path, &current_directory_glob_path)
                        && self
                            .search_one_file(
                                &resolved,
                                &current_directory_relative,
                                regex,
                                context_lines,
                                max_results,
                                &mut output,
                                false,
                            )
                            .await?
                    {
                        break 'search;
                    }
                }
            }
        }

        output.matched_files = output.files.len();

        Ok(output)
    }

    #[allow(clippy::too_many_arguments)]
    async fn search_one_file(
        &self,
        file: &Path,
        current_directory_relative: &Path,
        regex: &Regex,
        context_lines: usize,
        max_results: usize,
        output: &mut SearchTextOutput,
        binary_is_error: bool,
    ) -> Result<bool, ToolExecutionError> {
        let display_path = path_for_output(current_directory_relative);
        let content = read_text_content(file, &display_path).await?;
        let TextContent::Text(content) = content else {
            if binary_is_error {
                return Err(coded_error(
                    ToolErrorKind::InvalidArgs,
                    "BINARY_FILE",
                    "Text search cannot be performed on this file.",
                ));
            }

            output.skipped_binary_files += 1;
            return Ok(false);
        };
        let remaining = max_results.saturating_sub(output.matched_occurrences);
        let content_matches = find_matches(&content, regex, context_lines, remaining);

        if !content_matches.matches.is_empty() {
            output.matched_occurrences += content_matches.matches.len();
            output.files.push(FileMatches {
                path: display_path,
                matches: content_matches.matches,
            });
        }

        if content_matches.has_more {
            output.truncated = true;
            return Ok(true);
        }

        Ok(false)
    }
}

impl Tool for SearchText {
    const NAME: &'static str = "search_text";
    type Args = SearchTextArgs;
    type Output = SearchTextOutput;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Search text in a project file or recursively in a directory.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Literal text or regular expression to search for"
                },
                "path": {
                    "type": "string",
                    "minLength": 1,
                    "description": "current directory-relative file or directory path"
                },
                "mode": {
                    "type": "string",
                    "enum": [
                        "literal",
                        "regex"
                    ],
                    "default": "literal",
                    "description": "How to read query: exact text (literal) or a regular expression (regex)"
                },
                "case_sensitive": {
                    "type": "boolean",
                    "default": true,
                    "description": "Whether uppercase and lowercase letters must match exactly"
                },
                "include": {
                    "type": "array",
                    "items": {
                        "type": "string",
                        "minLength": 1
                    },
                    "description": "OR glob patterns relative to path. '*' does not cross '/'; '**' does. For files at every directory depth, prefix each pattern with '**/' (for example, '**/*.txt'). Without '**/', '*.txt' matches only files directly inside path."
                },
                "exclude": {
                    "type": "array",
                    "items": {
                        "type": "string",
                        "minLength": 1
                    },
                    "description": "OR glob patterns relative to path. '*' does not cross '/'; '**' does. To exclude a name at every directory depth, prefix the pattern with '**/' (for example, '**/generated/**')."
                },
                "context_lines": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 10,
                    "default": 2,
                    "description": "Number of lines to return before and after each matching line"
                },
                "max_results": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 200,
                    "default": 50,
                    "description": "Maximum total number of matching occurrences to return across all files"
                }
            },
            "required": [
                "query",
                "path"
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

        let regex = compile_query(&args.query, args.mode, args.case_sensitive)?;
        let filters = SearchFilters::new(args.include.as_deref(), args.exclude.as_deref())?;
        let (resolved, current_directory_relative) = self.resolve_path(&args.path).await?;
        let metadata = fs::metadata(&resolved)
            .await
            .map_err(|error| search_io_error(&args.path, "inspecting the search path", error))?;

        if metadata.is_file() {
            let search_relative = current_directory_relative
                .file_name()
                .map(PathBuf::from)
                .unwrap_or_else(|| current_directory_relative.clone());
            let search_glob_path = path_for_glob(&search_relative);
            let current_directory_glob_path = path_for_glob(&current_directory_relative);
            let mut output = SearchTextOutput::default();

            if filters.should_search_file(&search_glob_path, &current_directory_glob_path) {
                self.search_one_file(
                    &resolved,
                    &current_directory_relative,
                    &regex,
                    args.context_lines,
                    args.max_results,
                    &mut output,
                    true,
                )
                .await?;
            }

            output.matched_files = output.files.len();

            Ok(output)
        } else if metadata.is_dir() {
            self.search_directory(
                resolved,
                current_directory_relative,
                &regex,
                &filters,
                args.context_lines,
                args.max_results,
            )
            .await
        } else {
            Err(coded_error(
                ToolErrorKind::InvalidArgs,
                "UNSUPPORTED_PATH",
                "Search path is neither a directory nor a text file.",
            ))
        }
    }
}

impl SearchFilters {
    fn new(
        include: Option<&[String]>,
        exclude: Option<&[String]>,
    ) -> Result<Self, ToolExecutionError> {
        let default_patterns = DEFAULT_EXCLUDES
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(str::to_string)
            .collect::<Vec<_>>();

        Ok(Self {
            include: include
                .map(|patterns| GlobPatterns::compile(patterns, "include"))
                .transpose()?,
            exclude: GlobPatterns::compile(exclude.unwrap_or(&[]), "exclude")?,
            default_exclude: GlobPatterns::compile(&default_patterns, "default exclude")?,
        })
    }

    fn should_search_file(&self, search_relative: &str, current_directory_relative: &str) -> bool {
        let included = self
            .include
            .as_ref()
            .is_none_or(|patterns| patterns.matches(search_relative));

        included
            && !self.exclude.matches(search_relative)
            && !self.default_exclude.matches(current_directory_relative)
    }

    fn should_exclude_directory(
        &self,
        search_relative: &str,
        current_directory_relative: &str,
    ) -> bool {
        self.exclude.matches_directory(search_relative)
            || self
                .default_exclude
                .matches_directory(current_directory_relative)
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
                ));
            }

            compiled.push(Pattern::new(pattern).map_err(|error| {
                coded_error(
                    ToolErrorKind::InvalidArgs,
                    "INVALID_GLOB",
                    format!("Invalid {argument} glob \"{pattern}\": {error}"),
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

    fn matches_directory(&self, path: &str) -> bool {
        if self.matches(path) {
            return true;
        }

        let path = path.trim_end_matches('/');
        self.matches(&format!("{path}/")) || self.matches(&format!("{path}/__rigel_descendant__"))
    }
}

fn default_case_sensitive() -> bool {
    true
}

fn default_context_lines() -> usize {
    DEFAULT_CONTEXT_LINES
}

fn default_max_results() -> usize {
    DEFAULT_MAX_RESULTS
}

fn validate_args(args: &SearchTextArgs) -> Result<(), ToolExecutionError> {
    if args.query.is_empty() {
        return Err(coded_error(
            ToolErrorKind::InvalidArgs,
            "INVALID_ARGUMENT",
            "Search query must not be empty.",
        ));
    }

    if args.context_lines > 10 {
        return Err(coded_error(
            ToolErrorKind::InvalidArgs,
            "INVALID_ARGUMENT",
            "context_lines must be between 0 and 10.",
        ));
    }

    if !(1..=200).contains(&args.max_results) {
        return Err(coded_error(
            ToolErrorKind::InvalidArgs,
            "INVALID_ARGUMENT",
            "max_results must be between 1 and 200.",
        ));
    }

    Ok(())
}

fn compile_query(
    query: &str,
    mode: SearchMode,
    case_sensitive: bool,
) -> Result<Regex, ToolExecutionError> {
    let pattern = match mode {
        SearchMode::Literal => regex::escape(query),
        SearchMode::Regex => query.to_string(),
    };

    RegexBuilder::new(&pattern)
        .case_insensitive(!case_sensitive)
        .build()
        .map_err(|error| {
            coded_error(
                ToolErrorKind::InvalidArgs,
                "INVALID_REGEX",
                error.to_string(),
            )
        })
}

fn find_matches(
    content: &str,
    regex: &Regex,
    context_lines: usize,
    limit: usize,
) -> ContentMatches {
    let mut lines = content.lines().collect::<Vec<_>>();

    if lines.is_empty() {
        lines.push("");
    }

    let mut matches = Vec::new();

    for found in regex.find_iter(content) {
        if matches.len() == limit {
            return ContentMatches {
                matches,
                has_more: true,
            };
        }

        let prefix = &content[..found.start()];
        let mut line_index = prefix.bytes().filter(|byte| *byte == b'\n').count();
        line_index = line_index.min(lines.len() - 1);
        let line_start = prefix.rfind('\n').map_or(0, |index| index + 1);
        let column = content[line_start..found.start()].chars().count() + 1;
        let before_start = line_index.saturating_sub(context_lines);
        let after_end = (line_index + 1 + context_lines).min(lines.len());
        let before = (before_start..line_index)
            .map(|index| ContextLine {
                line: index + 1,
                text: lines[index].to_string(),
            })
            .collect();
        let after = (line_index + 1..after_end)
            .map(|index| ContextLine {
                line: index + 1,
                text: lines[index].to_string(),
            })
            .collect();

        matches.push(TextMatch {
            line: line_index + 1,
            column,
            text: lines[line_index].to_string(),
            before,
            after,
        });
    }

    ContentMatches {
        matches,
        has_more: false,
    }
}

async fn read_text_content(
    path: &Path,
    display_path: &str,
) -> Result<TextContent, ToolExecutionError> {
    let bytes = fs::read(path)
        .await
        .map_err(|error| search_io_error(display_path, "reading a file", error))?;

    Ok(decode_text_content(bytes))
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

fn search_io_error(path: &str, operation: &str, error: std::io::Error) -> ToolExecutionError {
    let kind = match error.kind() {
        std::io::ErrorKind::NotFound => ToolErrorKind::NotFound,
        std::io::ErrorKind::PermissionDenied => ToolErrorKind::PermissionDenied,
        _ => ToolErrorKind::Other,
    };
    let code = match error.kind() {
        std::io::ErrorKind::NotFound => "PATH_NOT_FOUND",
        std::io::ErrorKind::PermissionDenied => "PERMISSION_DENIED",
        _ => "IO_ERROR",
    };
    let message = match error.kind() {
        std::io::ErrorKind::NotFound => {
            format!("Cannot search \"{path}\" while {operation}: path no longer exists.")
        }
        std::io::ErrorKind::PermissionDenied => {
            format!("Cannot search \"{path}\" while {operation}: permission denied.")
        }
        _ => format!("Cannot search \"{path}\" while {operation}: {error}"),
    };

    coded_error(kind, code, message).with_source(error)
}

fn resolve_path_error(path: &str, error: std::io::Error) -> ToolExecutionError {
    if error.kind() == std::io::ErrorKind::NotFound {
        return coded_error(
            ToolErrorKind::NotFound,
            "PATH_NOT_FOUND",
            "Search path does not exist.",
        )
        .with_source(error);
    }

    search_io_error(path, "resolving the search path", error)
}

fn outside_current_directory_error(path: &str) -> ToolExecutionError {
    let message = format!("Search path \"{path}\" resolves outside the current directory.");
    with_coded_output(
        ToolExecutionError::refused(message.clone()),
        "PATH_OUTSIDE_CURRENT_DIRECTORY",
        message,
    )
}

fn coded_error(
    kind: ToolErrorKind,
    code: &'static str,
    message: impl Into<String>,
) -> ToolExecutionError {
    let message = message.into();
    with_coded_output(
        ToolExecutionError::new(kind, message.clone()),
        code,
        message,
    )
}

fn with_coded_output(
    error: ToolExecutionError,
    code: &'static str,
    message: String,
) -> ToolExecutionError {
    error
        .with_code(code)
        .with_model_output(ToolOutput::json(serde_json::json!({
            "code": code,
            "message": message,
        })))
}

fn glob_options() -> MatchOptions {
    MatchOptions {
        case_sensitive: true,
        require_literal_separator: true,
        require_literal_leading_dot: false,
    }
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
    let path = path_for_glob(path);
    if path.is_empty() {
        ".".to_string()
    } else {
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_contract() {
        let args: SearchTextArgs = serde_json::from_value(serde_json::json!({
            "query": "AppState",
            "path": "src"
        }))
        .expect("arguments should deserialize");

        assert_eq!(args.mode, SearchMode::Literal);
        assert!(args.case_sensitive);
        assert_eq!(args.context_lines, 2);
        assert_eq!(args.max_results, 50);
    }

    #[test]
    fn literal_search_returns_line_column_and_context() {
        let regex =
            compile_query("AppState", SearchMode::Literal, true).expect("query should compile");
        let result = find_matches(
            "#[derive(Debug)]\n#[derive(Default)]\npub struct AppState {\n    value: usize,\n}",
            &regex,
            2,
            50,
        );

        assert!(!result.has_more);
        assert_eq!(
            result.matches,
            vec![TextMatch {
                line: 3,
                column: 12,
                text: "pub struct AppState {".to_string(),
                before: vec![
                    ContextLine {
                        line: 1,
                        text: "#[derive(Debug)]".to_string(),
                    },
                    ContextLine {
                        line: 2,
                        text: "#[derive(Default)]".to_string(),
                    },
                ],
                after: vec![
                    ContextLine {
                        line: 4,
                        text: "    value: usize,".to_string(),
                    },
                    ContextLine {
                        line: 5,
                        text: "}".to_string(),
                    },
                ],
            }]
        );
    }

    #[test]
    fn regex_search_can_ignore_case() {
        let regex =
            compile_query(r"app\w+", SearchMode::Regex, false).expect("regex should compile");
        let result = find_matches("AppState\nAPP_CONFIG", &regex, 0, 50);

        assert_eq!(result.matches.len(), 2);
    }

    #[test]
    fn max_results_limits_occurrences_and_sets_truncated() {
        let regex = compile_query("x", SearchMode::Literal, true).expect("query should compile");
        let result = find_matches("x x x", &regex, 0, 2);

        assert_eq!(result.matches.len(), 2);
        assert!(result.has_more);
    }

    #[test]
    fn invalid_regex_has_structured_error_code() {
        let error =
            compile_query("[abc", SearchMode::Regex, true).expect_err("invalid regex should fail");

        assert_eq!(error.code(), Some("INVALID_REGEX"));
        assert_eq!(error.kind(), ToolErrorKind::InvalidArgs);
    }

    #[test]
    fn include_patterns_use_or_logic() {
        let patterns =
            GlobPatterns::compile(&["**/*.rs".to_string(), "**/*.toml".to_string()], "include")
                .expect("patterns should compile");

        assert!(patterns.matches("main.rs"));
        assert!(patterns.matches("src/lib.rs"));
        assert!(patterns.matches("Cargo.toml"));
        assert!(!patterns.matches("README.md"));
    }

    #[test]
    fn default_excludes_prune_known_directories() {
        let filters = SearchFilters::new(None, None).expect("filters should compile");

        assert!(filters.should_exclude_directory("target", "target"));
        assert!(filters.should_exclude_directory(".git", ".git"));
        assert!(filters.should_exclude_directory("node_modules", "node_modules"));
    }

    #[test]
    fn binary_heuristic_detects_nul_and_invalid_utf8() {
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
    fn outside_path_has_structured_error_code() {
        let error =
            SearchText::normalize_relative_path("../outside").expect_err("path should fail");

        assert_eq!(error.code(), Some("PATH_OUTSIDE_CURRENT_DIRECTORY"));
        assert!(error.is_refusal());
    }
}
