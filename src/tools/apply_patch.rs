use std::{
    env,
    path::{Component, Path, PathBuf},
};

use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::tools::revision::sha256;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ApplyPatchArgs {
    path: String,
    expected_revision: String,
    edits: Vec<TextEdit>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TextEdit {
    old_text: String,
    new_text: String,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ApplyPatchOutput {
    path: String,
    previous_revision: String,
    revision: String,
    applied_edits: usize,
    message: String,
}

struct PlannedEdit {
    index: usize,
    start: usize,
    end: usize,
}

#[derive(Debug)]
struct PreparedUpdate {
    content: String,
    previous_revision: String,
    revision: String,
    applied_edits: usize,
}

pub(crate) struct ApplyPatch {
    root: PathBuf,
}

impl ApplyPatch {
    pub(crate) async fn new() -> Result<Self, ToolExecutionError> {
        let current_dir = env::current_dir().map_err(|error| {
            ToolExecutionError::other(format!("Cannot determine the project root: {error}"))
                .with_source(error)
        })?;
        let root = fs::canonicalize(&current_dir).await.map_err(|error| {
            ToolExecutionError::other(format!(
                "Cannot access the project root \"{}\": {error}",
                current_dir.display()
            ))
            .with_source(error)
        })?;

        Ok(Self { root })
    }

    fn normalize_relative_path(path: &str) -> Result<PathBuf, ToolExecutionError> {
        if path.is_empty() {
            return Err(ToolExecutionError::invalid_args(
                "Cannot update file: path must not be empty.",
            ));
        }

        let mut normalized = PathBuf::new();

        for component in Path::new(path).components() {
            match component {
                Component::Normal(component) => normalized.push(component),
                Component::ParentDir if !normalized.pop() => {
                    return Err(outside_project_error(path));
                }
                Component::ParentDir | Component::CurDir => {}
                Component::RootDir | Component::Prefix(_) => {
                    return Err(ToolExecutionError::invalid_args(format!(
                        "Cannot update file \"{path}\": path must be relative to the project root."
                    )));
                }
            }
        }

        if normalized.as_os_str().is_empty() {
            return Err(ToolExecutionError::invalid_args(format!(
                "Cannot update file \"{path}\": path must name a file."
            )));
        }

        Ok(normalized)
    }

    async fn resolve_path(&self, path: &str) -> Result<(PathBuf, String), ToolExecutionError> {
        let relative_path = Self::normalize_relative_path(path)?;
        let resolved = fs::canonicalize(self.root.join(&relative_path))
            .await
            .map_err(|error| update_io_error(path, "resolving the file", error))?;

        if !resolved.starts_with(&self.root) {
            return Err(outside_project_error(path));
        }

        Ok((resolved, relative_path.to_string_lossy().into_owned()))
    }
}

impl Tool for ApplyPatch {
    const NAME: &'static str = "apply_patch";
    type Args = ApplyPatchArgs;
    type Output = ApplyPatchOutput;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Update an existing UTF-8 file with revision-checked text edits that tolerate whitespace differences. The returned revision must be used as expected_revision for the next patch to this file."
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Current directory-relative path to an existing UTF-8 text file"
                },
                "expected_revision": {
                    "type": "string",
                    "description": "Revision returned by read_file or by the previous successful apply_patch call. After a successful patch, use the returned revision for the next patch to this file; never reuse an older revision."
                },
                "edits": {
                    "type": "array",
                    "minItems": 1,
                    "description": "Text replacements to apply. Every old_text is matched against the same original file, and edits must not overlap. Matching tolerates whitespace differences.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "old_text": {
                                "type": "string",
                                "minLength": 1,
                                "description": "Text that must occur exactly once; matched exactly first, then runs of whitespace are treated as a single space"
                            },
                            "new_text": {
                                "type": "string",
                                "description": "Replacement text; empty string deletes old_text"
                            }
                        },
                        "required": [
                            "old_text",
                            "new_text"
                        ],
                        "additionalProperties": false
                    }
                }
            },
            "required": [
                "path",
                "expected_revision",
                "edits"
            ],
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
            return Err(ToolExecutionError::invalid_args(format!(
                "Cannot update file \"{}\": path is not a regular file.",
                args.path
            )));
        }

        let original_bytes = fs::read(&resolved)
            .await
            .map_err(|error| update_io_error(&args.path, "reading the file", error))?;
        let original = String::from_utf8(original_bytes).map_err(|error| {
            ToolExecutionError::invalid_args(format!(
                "Cannot update file \"{}\": file is not valid UTF-8 text.",
                args.path
            ))
            .with_source(error)
        })?;
        let prepared = prepare_update(&original, &args.expected_revision, &args.edits, &args.path)?;

        replace_file(
            &resolved,
            &prepared.content,
            &prepared.previous_revision,
            &args.path,
        )
        .await?;

        Ok(ApplyPatchOutput {
            path,
            previous_revision: prepared.previous_revision,
            revision: prepared.revision,
            applied_edits: prepared.applied_edits,
            message: "File was updated. Use this returned revision as expected_revision for the next patch to this file; call read_file again only when you need the current content.".to_string(),
        })
    }
}

fn prepare_update(
    original: &str,
    expected_revision: &str,
    edits: &[TextEdit],
    path: &str,
) -> Result<PreparedUpdate, ToolExecutionError> {
    let current_revision = sha256(original.as_bytes());

    if expected_revision != current_revision {
        return Err(revision_mismatch_error(
            path,
            expected_revision,
            &current_revision,
        ));
    }

    if edits.is_empty() {
        return Err(ToolExecutionError::invalid_args(format!(
            "Cannot update file \"{path}\": edits must contain at least one edit. No changes were made."
        )));
    }

    let mut planned_edits = Vec::with_capacity(edits.len());

    for (index, edit) in edits.iter().enumerate() {
        if edit.old_text.is_empty() {
            return Err(ToolExecutionError::invalid_args(format!(
                "Cannot apply edit {} to \"{path}\": old_text must not be empty. No changes were made.",
                index + 1
            )));
        }

        let (start, end) = match classify_occurrence(original, &edit.old_text) {
            OccurrenceMatch::Single { start, end } => (start, end),
            OccurrenceMatch::AmbiguousExact { count } => {
                return Err(ambiguous_exact_occurrence_error(path, index + 1, count));
            }
            OccurrenceMatch::AmbiguousWhitespace { count } => {
                return Err(ambiguous_whitespace_occurrence_error(
                    path,
                    index + 1,
                    count,
                ));
            }
            OccurrenceMatch::NotFound => {
                return Err(occurrence_not_found_error(path, index + 1));
            }
        };

        planned_edits.push(PlannedEdit { index, start, end });
    }

    planned_edits.sort_by_key(|edit| edit.start);

    for pair in planned_edits.windows(2) {
        let left = &pair[0];
        let right = &pair[1];

        if right.start < left.end {
            return Err(ToolExecutionError::invalid_args(format!(
                "Cannot update file \"{path}\": edits {} and {} overlap in the original text. Each edit must target separate text. No changes were made. Call read_file again and combine the overlapping edits.",
                left.index + 1,
                right.index + 1
            )));
        }
    }

    let mut updated = String::new();
    let mut cursor = 0_usize;

    for planned in &planned_edits {
        updated.push_str(&original[cursor..planned.start]);
        updated.push_str(&edits[planned.index].new_text);
        cursor = planned.end;
    }

    updated.push_str(&original[cursor..]);
    let revision = sha256(updated.as_bytes());

    Ok(PreparedUpdate {
        content: updated,
        previous_revision: current_revision,
        revision,
        applied_edits: edits.len(),
    })
}

fn occurrence_positions(text: &str, needle: &str) -> Vec<usize> {
    let mut positions = Vec::new();
    let mut search_start = 0_usize;

    while search_start <= text.len() {
        let Some(relative_position) = text[search_start..].find(needle) else {
            break;
        };
        let position = search_start + relative_position;
        positions.push(position);

        let character_length = text[position..].chars().next().map_or(1, char::len_utf8);
        search_start = position + character_length;
    }

    positions
}

/// Outcome of locating one edit's `old_text` in the original file.
enum OccurrenceMatch {
    Single { start: usize, end: usize },
    AmbiguousExact { count: usize },
    AmbiguousWhitespace { count: usize },
    NotFound,
}

/// Locates an edit's `old_text` exactly first, then tolerating whitespace
/// differences, and reports whether the occurrence is unique.
fn classify_occurrence(original: &str, old_text: &str) -> OccurrenceMatch {
    let exact = occurrence_positions(original, old_text);

    match exact.len() {
        0 => match fuzzy_occurrence_ranges(original, old_text).as_slice() {
            [] => OccurrenceMatch::NotFound,
            [(start, end)] => OccurrenceMatch::Single {
                start: *start,
                end: *end,
            },
            ranges => OccurrenceMatch::AmbiguousWhitespace {
                count: ranges.len(),
            },
        },
        1 => {
            let start = exact[0];
            OccurrenceMatch::Single {
                start,
                end: start + old_text.len(),
            }
        }
        count => OccurrenceMatch::AmbiguousExact { count },
    }
}

/// Collapses every run of whitespace in `text` into a single space and
/// records, for each normalized character, the byte range it occupies in the
/// original text so fuzzy matches can be mapped back onto the file.
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
            let end = start + character.len_utf8();
            normalized.push(character);
            spans.push((start, end));
        }
    }

    (normalized, spans)
}

/// Finds every byte range in `text` where `needle` occurs with runs of
/// whitespace treated as a single space. Overlapping occurrences are reported
/// separately, mirroring the exact matcher's scan.
fn fuzzy_occurrence_ranges(text: &str, needle: &str) -> Vec<(usize, usize)> {
    let (normalized, spans) = normalize_whitespace(text);
    let normalized_needle = normalize_whitespace(needle).0;

    if normalized_needle.is_empty() {
        return Vec::new();
    }

    let char_starts = normalized
        .char_indices()
        .map(|(offset, _)| offset)
        .collect::<Vec<usize>>();
    let mut ranges = Vec::new();
    let mut search_from = 0_usize;

    while let Some(relative) = normalized[search_from..].find(&normalized_needle) {
        let start_byte = search_from + relative;
        let end_byte = start_byte + normalized_needle.len();
        let start_index = char_starts.partition_point(|&offset| offset <= start_byte) - 1;
        let end_index = char_starts.partition_point(|&offset| offset < end_byte);
        ranges.push((spans[start_index].0, spans[end_index - 1].1));

        let character_length = normalized[start_byte..]
            .chars()
            .next()
            .map_or(1, char::len_utf8);
        search_from = start_byte + character_length;
    }

    ranges
}

async fn replace_file(
    target: &Path,
    content: &str,
    expected_revision: &str,
    display_path: &str,
) -> Result<(), ToolExecutionError> {
    let current_bytes = fs::read(target).await.map_err(|error| {
        update_io_error(display_path, "checking the revision before writing", error)
    })?;
    let current_revision = sha256(&current_bytes);

    if current_revision != expected_revision {
        return Err(revision_mismatch_error(
            display_path,
            expected_revision,
            &current_revision,
        ));
    }

    fs::write(target, content)
        .await
        .map_err(|error| update_io_error(display_path, "writing the updated file", error))
}

fn occurrence_not_found_error(path: &str, index: usize) -> ToolExecutionError {
    ToolExecutionError::invalid_args(format!(
        "Cannot apply edit {index} to \"{path}\": old_text was not found in the original file, even when whitespace differences are ignored. No changes were made. Call read_file again and provide more surrounding context."
    ))
}

fn ambiguous_exact_occurrence_error(path: &str, index: usize, count: usize) -> ToolExecutionError {
    ToolExecutionError::invalid_args(format!(
        "Cannot apply edit {index} to \"{path}\": old_text occurs {count} times in the original file and must occur exactly once. No changes were made. Call read_file again and provide more surrounding context."
    ))
}

fn ambiguous_whitespace_occurrence_error(
    path: &str,
    index: usize,
    count: usize,
) -> ToolExecutionError {
    ToolExecutionError::invalid_args(format!(
        "Cannot apply edit {index} to \"{path}\": old_text occurs {count} times in the original file when whitespace differences are ignored, and must occur exactly once. No changes were made. Call read_file again and provide more surrounding context."
    ))
}

fn revision_mismatch_error(
    path: &str,
    expected_revision: &str,
    current_revision: &str,
) -> ToolExecutionError {
    ToolExecutionError::invalid_args(format!(
        "Cannot update file \"{path}\": expected_revision \"{expected_revision}\" does not match the current revision \"{current_revision}\". No edits were committed. Use the current revision from this error or call read_file, then retry."
    ))
}

fn outside_project_error(path: &str) -> ToolExecutionError {
    ToolExecutionError::refused(format!(
        "Cannot update file \"{path}\": path resolves outside the project root."
    ))
}

fn update_io_error(path: &str, operation: &str, error: std::io::Error) -> ToolExecutionError {
    let message = match error.kind() {
        std::io::ErrorKind::NotFound => {
            format!("Cannot update file \"{path}\" while {operation}: file does not exist.")
        }
        std::io::ErrorKind::PermissionDenied => {
            format!("Cannot update file \"{path}\" while {operation}: permission denied.")
        }
        std::io::ErrorKind::NotADirectory => {
            format!(
                "Cannot update file \"{path}\" while {operation}: a path component is not a directory."
            )
        }
        _ => format!("Cannot update file \"{path}\" while {operation}: {error}"),
    };

    match error.kind() {
        std::io::ErrorKind::NotFound => ToolExecutionError::not_found(message),
        std::io::ErrorKind::PermissionDenied => ToolExecutionError::permission_denied(message),
        std::io::ErrorKind::NotADirectory => ToolExecutionError::invalid_args(message),
        _ => ToolExecutionError::other(message),
    }
    .with_source(error)
}

#[cfg(test)]
mod tests {
    use rig::tool::ToolErrorKind;

    use super::*;

    fn edit(old_text: &str, new_text: &str) -> TextEdit {
        TextEdit {
            old_text: old_text.to_string(),
            new_text: new_text.to_string(),
        }
    }

    #[test]
    fn edits_are_isolated_against_original_text() {
        let original = "alpha beta";
        let revision = sha256(original.as_bytes());
        let prepared = prepare_update(
            original,
            &revision,
            &[edit("alpha", "beta"), edit("beta", "gamma")],
            "file.txt",
        )
        .expect("non-overlapping edits should apply");

        assert_eq!(prepared.content, "beta gamma");
        assert_eq!(prepared.applied_edits, 2);
    }

    #[test]
    fn multiple_old_text_occurrences_are_a_clear_error() {
        let original = "one two one";
        let revision = sha256(original.as_bytes());
        let error = prepare_update(original, &revision, &[edit("one", "three")], "file.txt")
            .expect_err("ambiguous edit should fail");

        assert_eq!(error.kind(), ToolErrorKind::InvalidArgs);
        assert_eq!(
            error.model_feedback(),
            Some(
                "Cannot apply edit 1 to \"file.txt\": old_text occurs 2 times in the original file and must occur exactly once. No changes were made. Call read_file again and provide more surrounding context."
            )
        );
    }

    #[test]
    fn overlapping_occurrences_are_counted() {
        assert_eq!(occurrence_positions("aaa", "aa"), vec![0, 1]);
    }

    #[test]
    fn revision_mismatch_reports_current_revision() {
        let error = prepare_update(
            "current",
            "stale",
            &[edit("current", "updated")],
            "file.txt",
        )
        .expect_err("stale revision should fail");

        assert_eq!(error.kind(), ToolErrorKind::InvalidArgs);
        assert!(
            error
                .model_feedback()
                .is_some_and(|message| message.contains("Use the current revision from this error"))
        );
    }

    #[test]
    fn overlapping_edits_fail_before_changes() {
        let original = "abcdef";
        let revision = sha256(original.as_bytes());
        let error = prepare_update(
            original,
            &revision,
            &[edit("abc", "x"), edit("cde", "y")],
            "file.txt",
        )
        .expect_err("overlapping edits should fail");

        assert_eq!(error.kind(), ToolErrorKind::InvalidArgs);
        assert!(
            error
                .model_feedback()
                .is_some_and(|message| message.contains("edits 1 and 2 overlap"))
        );
    }

    #[test]
    fn successful_update_reports_revisions() {
        let original = "hello world";
        let previous_revision = sha256(original.as_bytes());
        let prepared = prepare_update(
            original,
            &previous_revision,
            &[edit("world", "Rust!")],
            "file.txt",
        )
        .expect("edit should apply");

        assert_eq!(prepared.previous_revision, previous_revision);
        assert_eq!(prepared.revision, sha256(b"hello Rust!"));
    }

    #[test]
    fn outside_path_is_refused() {
        let error = ApplyPatch::normalize_relative_path("../outside")
            .expect_err("outside path should fail");

        assert!(error.is_refusal());
        assert_eq!(
            error.model_feedback(),
            Some("Cannot update file \"../outside\": path resolves outside the project root.")
        );
    }

    #[test]
    fn whitespace_differences_are_tolerated() {
        let original = "alpha  beta gamma";
        let revision = sha256(original.as_bytes());
        let prepared = prepare_update(
            original,
            &revision,
            &[edit("beta gamma", "beta")],
            "file.txt",
        )
        .expect("fuzzy edit should apply");

        assert_eq!(prepared.content, "alpha  beta");
    }

    #[test]
    fn exact_match_is_preferred_over_fuzzy() {
        let original = "a b  c";
        let revision = sha256(original.as_bytes());
        let prepared = prepare_update(original, &revision, &[edit("a b", "x")], "file.txt")
            .expect("exact edit should apply");

        assert_eq!(prepared.content, "x  c");
    }

    #[test]
    fn line_endings_are_ignored() {
        let original = "foo\r\nbar\r\n";
        let revision = sha256(original.as_bytes());
        let prepared = prepare_update(
            original,
            &revision,
            &[edit("foo\nbar\n", "baz")],
            "file.txt",
        )
        .expect("fuzzy edit should apply");

        assert_eq!(prepared.content, "baz");
    }

    #[test]
    fn tabs_and_indentation_are_ignored() {
        let original = "fn main() {\n    call();\n}\n";
        let revision = sha256(original.as_bytes());
        let prepared = prepare_update(
            original,
            &revision,
            &[edit("fn main() {\n\tcall();\n}", "fn main() {}")],
            "file.txt",
        )
        .expect("fuzzy edit should apply");

        assert_eq!(prepared.content, "fn main() {}\n");
    }

    #[test]
    fn unicode_whitespace_is_tolerated() {
        let original = "привет\u{00A0}мир";
        let revision = sha256(original.as_bytes());
        let prepared = prepare_update(
            original,
            &revision,
            &[edit("привет мир", "hello world")],
            "file.txt",
        )
        .expect("fuzzy edit should apply");

        assert_eq!(prepared.content, "hello world");
    }

    #[test]
    fn fuzzy_match_spans_the_whole_whitespace_run() {
        let original = "foo    bar";
        let revision = sha256(original.as_bytes());
        let prepared = prepare_update(
            original,
            &revision,
            &[edit("foo bar", "foobar")],
            "file.txt",
        )
        .expect("fuzzy edit should apply");

        assert_eq!(prepared.content, "foobar");
    }

    #[test]
    fn fuzzy_match_covering_the_whole_file_is_mapped_correctly() {
        let original = "foo bar";
        let revision = sha256(original.as_bytes());
        let prepared = prepare_update(original, &revision, &[edit("foo\nbar", "baz")], "file.txt")
            .expect("fuzzy edit should apply");

        assert_eq!(prepared.content, "baz");
    }

    #[test]
    fn fuzzy_ambiguity_is_a_clear_error() {
        let original = "a b a b";
        let revision = sha256(original.as_bytes());
        let error = prepare_update(original, &revision, &[edit("a  b", "x")], "file.txt")
            .expect_err("ambiguous fuzzy edit should fail");

        assert_eq!(error.kind(), ToolErrorKind::InvalidArgs);
        assert_eq!(
            error.model_feedback(),
            Some(
                "Cannot apply edit 1 to \"file.txt\": old_text occurs 2 times in the original file when whitespace differences are ignored, and must occur exactly once. No changes were made. Call read_file again and provide more surrounding context."
            )
        );
    }

    #[test]
    fn exact_ambiguity_keeps_the_exact_error_message() {
        let original = "a b a b";
        let revision = sha256(original.as_bytes());
        let error = prepare_update(original, &revision, &[edit("a b", "x")], "file.txt")
            .expect_err("ambiguous edit should fail");

        assert_eq!(error.kind(), ToolErrorKind::InvalidArgs);
        assert_eq!(
            error.model_feedback(),
            Some(
                "Cannot apply edit 1 to \"file.txt\": old_text occurs 2 times in the original file and must occur exactly once. No changes were made. Call read_file again and provide more surrounding context."
            )
        );
    }

    #[test]
    fn overlapping_fuzzy_occurrences_are_counted() {
        let original = "a  a  a";
        let revision = sha256(original.as_bytes());
        let error = prepare_update(original, &revision, &[edit("a a", "x")], "file.txt")
            .expect_err("overlapping fuzzy matches should fail");

        assert_eq!(error.kind(), ToolErrorKind::InvalidArgs);
        assert!(error.model_feedback().is_some_and(|message| {
            message.contains(
                "old_text occurs 2 times in the original file when whitespace differences are ignored"
            )
        }));
    }

    #[test]
    fn fuzzy_not_found_reports_whitespace_insensitive_search() {
        let original = "alpha beta";
        let revision = sha256(original.as_bytes());
        let error = prepare_update(original, &revision, &[edit("gamma delta", "x")], "file.txt")
            .expect_err("missing text should fail");

        assert_eq!(error.kind(), ToolErrorKind::InvalidArgs);
        assert_eq!(
            error.model_feedback(),
            Some(
                "Cannot apply edit 1 to \"file.txt\": old_text was not found in the original file, even when whitespace differences are ignored. No changes were made. Call read_file again and provide more surrounding context."
            )
        );
    }

    #[test]
    fn fuzzy_edits_cannot_overlap_in_the_original_text() {
        let original = "x a  b c";
        let revision = sha256(original.as_bytes());
        let error = prepare_update(
            original,
            &revision,
            &[edit("a b", "d"), edit("b c", "e")],
            "file.txt",
        )
        .expect_err("overlapping edits should fail");

        assert_eq!(error.kind(), ToolErrorKind::InvalidArgs);
        assert!(
            error
                .model_feedback()
                .is_some_and(|message| message.contains("edits 1 and 2 overlap"))
        );
    }

    #[test]
    fn edits_are_isolated_against_original_text_with_fuzzy_matching() {
        let original = "a  b";
        let revision = sha256(original.as_bytes());
        let error = prepare_update(
            original,
            &revision,
            &[edit("a b", "x y"), edit("y", "z")],
            "file.txt",
        )
        .expect_err("edit 2 must be matched against the original file");

        assert_eq!(error.kind(), ToolErrorKind::InvalidArgs);
        assert!(
            error
                .model_feedback()
                .is_some_and(|message| message.contains("Cannot apply edit 2 to \"file.txt\""))
        );
    }

    #[test]
    fn whitespace_only_old_text_matches_a_single_whitespace_run() {
        let original = "a b";
        let revision = sha256(original.as_bytes());
        let prepared = prepare_update(original, &revision, &[edit("  ", ",")], "file.txt")
            .expect("whitespace-only edit should apply");

        assert_eq!(prepared.content, "a,b");
    }
}
