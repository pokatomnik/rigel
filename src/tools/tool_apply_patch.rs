use std::{
    env,
    path::{Component, Path, PathBuf},
};

use rig::tool::{Tool, ToolContext, ToolErrorKind, ToolExecutionError};
use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::tools::{
    contracts::{Action, error_codes},
    revision::sha256,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ApplyPatchArgs {
    path: String,
    revision: String,
    find: String,
    replace: String,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ApplyPatchOutput {
    action: Action,
    path: String,
    revision: String,
}

#[derive(Debug, PartialEq, Eq)]
struct PreparedUpdate {
    content: String,
    revision: String,
}

enum OccurrenceMatch {
    Single { start: usize, end: usize },
    Ambiguous { count: usize },
    NotFound,
}

pub(crate) struct ApplyPatch {
    root: PathBuf,
}

impl ApplyPatch {
    pub(crate) async fn new() -> Result<Self, ToolExecutionError> {
        let current_dir = env::current_dir().map_err(|error| {
            ToolExecutionError::other(format!("Cannot determine the current directory: {error}"))
                .with_code(error_codes::IO_ERROR)
                .with_source(error)
        })?;
        let root = fs::canonicalize(&current_dir).await.map_err(|error| {
            ToolExecutionError::other(format!(
                "Cannot access the current directory \"{}\": {error}",
                current_dir.display()
            ))
            .with_code(error_codes::IO_ERROR)
            .with_source(error)
        })?;
        Ok(Self { root })
    }

    fn normalize_relative_path(path: &str) -> Result<PathBuf, ToolExecutionError> {
        if path.is_empty() {
            return Err(coded_error(
                error_codes::INVALID_ARGUMENT,
                "Cannot update file: path must not be empty.",
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
        if normalized.as_os_str().is_empty() {
            return Err(coded_error(
                error_codes::INVALID_ARGUMENT,
                format!("Cannot update file \"{path}\": path must name a file."),
            ));
        }
        Ok(normalized)
    }

    async fn resolve_path(&self, path: &str) -> Result<(PathBuf, String), ToolExecutionError> {
        let relative = Self::normalize_relative_path(path)?;
        let resolved = fs::canonicalize(self.root.join(&relative))
            .await
            .map_err(|error| update_io_error(path, "resolving the file", error))?;
        if !resolved.starts_with(&self.root) {
            return Err(outside_current_directory_error(path));
        }
        Ok((resolved, relative.to_string_lossy().into_owned()))
    }
}

impl Tool for ApplyPatch {
    const NAME: &'static str = "apply_patch";
    type Args = ApplyPatchArgs;
    type Output = ApplyPatchOutput;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Replace one exact text occurrence in an existing UTF-8 file using its revision."
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Existing UTF-8 file path relative to the current directory."
                },
                "revision": {
                    "type": "string",
                    "description": "Revision returned by read_file or the previous successful apply_patch."
                },
                "find": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Text to find exactly once. Whitespace differences are tolerated only when exact text is absent."
                },
                "replace": {
                    "type": "string",
                    "description": "Replacement text. An empty string deletes the found text."
                }
            },
            "required": ["path", "revision", "find", "replace"],
            "additionalProperties": false
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let (resolved, path) = self.resolve_path(&args.path).await?;
        let metadata = fs::metadata(&resolved)
            .await
            .map_err(|error| update_io_error(&args.path, "reading file metadata", error))?;
        if !metadata.is_file() {
            return Err(coded_error(
                error_codes::INVALID_PATH_TYPE,
                format!(
                    "Cannot update file \"{}\": path is not a regular file.",
                    args.path
                ),
            ));
        }
        let original_bytes = fs::read(&resolved)
            .await
            .map_err(|error| update_io_error(&args.path, "reading the file", error))?;
        let original = String::from_utf8(original_bytes).map_err(|error| {
            coded_error(
                error_codes::BINARY_FILE,
                format!(
                    "Cannot update file \"{}\": file is not valid UTF-8 text.",
                    args.path
                ),
            )
            .with_source(error)
        })?;
        let prepared = prepare_update(&original, &args)?;
        write_updated_file(&resolved, &prepared.content, &args.revision, &args.path).await?;
        Ok(ApplyPatchOutput {
            action: Action::Updated,
            path,
            revision: prepared.revision,
        })
    }
}

fn prepare_update(
    original: &str,
    args: &ApplyPatchArgs,
) -> Result<PreparedUpdate, ToolExecutionError> {
    let current_revision = sha256(original.as_bytes());
    if current_revision != args.revision {
        return Err(revision_changed_error(
            &args.path,
            &args.revision,
            &current_revision,
        ));
    }
    if args.find.is_empty() {
        return Err(coded_error(
            error_codes::INVALID_ARGUMENT,
            format!(
                "Cannot update file \"{}\": find must not be empty.",
                args.path
            ),
        ));
    }
    let (start, end) = match classify_occurrence(original, &args.find) {
        OccurrenceMatch::Single { start, end } => (start, end),
        OccurrenceMatch::Ambiguous { count } => {
            return Err(coded_error(
                error_codes::TEXT_NOT_UNIQUE,
                format!(
                    "Cannot update file \"{}\": find occurs {count} times.",
                    args.path
                ),
            ));
        }
        OccurrenceMatch::NotFound => {
            return Err(coded_error(
                error_codes::TEXT_NOT_FOUND,
                format!("Cannot update file \"{}\": find was not found.", args.path),
            ));
        }
    };
    let mut content = String::with_capacity(original.len() + args.replace.len());
    content.push_str(&original[..start]);
    content.push_str(&args.replace);
    content.push_str(&original[end..]);
    let revision = sha256(content.as_bytes());
    Ok(PreparedUpdate { content, revision })
}

fn classify_occurrence(original: &str, find: &str) -> OccurrenceMatch {
    let exact = occurrence_positions(original, find);
    match exact.len() {
        0 => {
            let fuzzy = fuzzy_occurrence_ranges(original, find);
            match fuzzy.as_slice() {
                [] => OccurrenceMatch::NotFound,
                [(start, end)] => OccurrenceMatch::Single {
                    start: *start,
                    end: *end,
                },
                ranges => OccurrenceMatch::Ambiguous {
                    count: ranges.len(),
                },
            }
        }
        1 => OccurrenceMatch::Single {
            start: exact[0],
            end: exact[0] + find.len(),
        },
        count => OccurrenceMatch::Ambiguous { count },
    }
}

async fn write_updated_file(
    target: &Path,
    content: &str,
    expected_revision: &str,
    display_path: &str,
) -> Result<(), ToolExecutionError> {
    let bytes = fs::read(target).await.map_err(|error| {
        update_io_error(display_path, "checking the file before writing", error)
    })?;
    let current_revision = sha256(&bytes);
    if current_revision != expected_revision {
        return Err(revision_changed_error(
            display_path,
            expected_revision,
            &current_revision,
        ));
    }
    fs::write(target, content.as_bytes())
        .await
        .map_err(|error| update_io_error(display_path, "writing the file", error))
}

fn occurrence_positions(text: &str, needle: &str) -> Vec<usize> {
    let mut positions = Vec::new();
    let mut search_start = 0;
    while search_start <= text.len() {
        let Some(relative) = text[search_start..].find(needle) else {
            break;
        };
        let position = search_start + relative;
        positions.push(position);
        let width = text[position..].chars().next().map_or(1, char::len_utf8);
        search_start = position + width;
    }
    positions
}

fn normalize_whitespace(text: &str) -> (String, Vec<(usize, usize)>) {
    let mut normalized = String::with_capacity(text.len());
    let mut spans = Vec::with_capacity(text.len());
    let mut characters = text.char_indices().peekable();
    while let Some((start, character)) = characters.next() {
        if character.is_whitespace() {
            let mut end = start + character.len_utf8();
            while let Some(&(next_start, next)) = characters.peek() {
                if !next.is_whitespace() {
                    break;
                }
                end = next_start + next.len_utf8();
                characters.next();
            }
            normalized.push(' ');
            spans.push((start, end));
        } else {
            normalized.push(character);
            spans.push((start, start + character.len_utf8()));
        }
    }
    (normalized, spans)
}

fn fuzzy_occurrence_ranges(text: &str, find: &str) -> Vec<(usize, usize)> {
    let (normalized, spans) = normalize_whitespace(text);
    let normalized_find = normalize_whitespace(find).0;
    if normalized_find.is_empty() {
        return Vec::new();
    }
    let starts = normalized
        .char_indices()
        .map(|(offset, _)| offset)
        .collect::<Vec<_>>();
    let mut ranges = Vec::new();
    let mut search_from = 0;
    while let Some(relative) = normalized[search_from..].find(&normalized_find) {
        let start = search_from + relative;
        let end = start + normalized_find.len();
        let start_index = starts.partition_point(|&offset| offset <= start) - 1;
        let end_index = starts.partition_point(|&offset| offset < end);
        ranges.push((spans[start_index].0, spans[end_index - 1].1));
        let width = normalized[start..].chars().next().map_or(1, char::len_utf8);
        search_from = start + width;
    }
    ranges
}

pub(crate) fn revision_changed_error(
    path: &str,
    expected: &str,
    current: &str,
) -> ToolExecutionError {
    coded_error(
        error_codes::REVISION_CHANGED,
        format!(
            "Cannot update file \"{path}\": revision \"{expected}\" is stale; current revision is \"{current}\". Read the file again and retry."
        ),
    )
}

fn outside_current_directory_error(path: &str) -> ToolExecutionError {
    ToolExecutionError::refused(format!(
        "Cannot update file \"{path}\": path is outside the current directory."
    ))
    .with_code(error_codes::PATH_OUTSIDE_CURRENT_DIRECTORY)
}

pub(crate) fn update_io_error(
    path: &str,
    operation: &str,
    error: std::io::Error,
) -> ToolExecutionError {
    let (kind, code) = match error.kind() {
        std::io::ErrorKind::NotFound => (ToolErrorKind::NotFound, error_codes::PATH_NOT_FOUND),
        std::io::ErrorKind::PermissionDenied => (
            ToolErrorKind::PermissionDenied,
            error_codes::PERMISSION_DENIED,
        ),
        _ => (ToolErrorKind::Other, error_codes::IO_ERROR),
    };
    ToolExecutionError::new(
        kind,
        format!("Cannot update file \"{path}\" while {operation}: {error}"),
    )
    .with_code(code)
    .with_source(error)
}

pub(crate) fn coded_error(code: &'static str, message: impl Into<String>) -> ToolExecutionError {
    ToolExecutionError::invalid_args(message).with_code(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(revision: &str, find: &str, replace: &str) -> ApplyPatchArgs {
        ApplyPatchArgs {
            path: "file.txt".to_string(),
            revision: revision.to_string(),
            find: find.to_string(),
            replace: replace.to_string(),
        }
    }

    #[test]
    fn schema_is_flat_and_has_one_replacement() {
        assert!(
            serde_json::from_value::<ApplyPatchArgs>(serde_json::json!({
                "path": "src/main.rs",
                "revision": "abc",
                "find": "old",
                "replace": "new"
            }))
            .is_ok()
        );
        assert!(
            serde_json::from_value::<ApplyPatchArgs>(serde_json::json!({
                "path": "src/main.rs",
                "revision": "abc",
                "edits": []
            }))
            .is_err()
        );
    }

    #[test]
    fn exact_match_is_preferred_and_replacement_can_delete() {
        let revision = sha256(b"a  b");
        let prepared = prepare_update("a  b", &args(&revision, "a  b", ""));

        assert!(prepared.is_ok());
        if let Ok(prepared) = prepared {
            assert_eq!(prepared.content, "");
            assert_eq!(prepared.revision, sha256(b""));
        }
    }

    #[test]
    fn fuzzy_matching_tolerates_whitespace_only_when_exact_is_absent() {
        let original = "alpha  beta";
        let revision = sha256(original.as_bytes());
        let prepared = prepare_update(original, &args(&revision, "alpha beta", "changed"));

        assert!(prepared.is_ok());
        if let Ok(prepared) = prepared {
            assert_eq!(prepared.content, "changed");
        }
    }

    #[test]
    fn missing_and_ambiguous_text_have_distinct_codes() {
        let revision = sha256(b"one two one");
        let missing = prepare_update("one two", &args(&sha256(b"one two"), "three", "x"));
        let ambiguous = prepare_update("one two one", &args(&revision, "one", "x"));

        assert_eq!(
            missing
                .err()
                .and_then(|error| error.code().map(str::to_string)),
            Some(error_codes::TEXT_NOT_FOUND.to_string())
        );
        assert_eq!(
            ambiguous
                .err()
                .and_then(|error| error.code().map(str::to_string)),
            Some(error_codes::TEXT_NOT_UNIQUE.to_string())
        );
    }

    #[test]
    fn stale_revision_is_rejected_before_planning() {
        let error = prepare_update("current", &args("stale", "current", "new"));

        assert!(error.is_err());
        if let Err(error) = error {
            assert_eq!(error.code(), Some(error_codes::REVISION_CHANGED));
        }
    }

    #[test]
    fn overlapping_occurrences_are_counted() {
        assert_eq!(occurrence_positions("aaa", "aa"), vec![0, 1]);
    }
}
