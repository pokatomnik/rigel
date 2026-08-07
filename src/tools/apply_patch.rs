use std::{
    env,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::{Deserialize, Serialize};
use tokio::{
    fs::{self, File, OpenOptions},
    io::AsyncWriteExt,
};

use crate::tools::revision::sha256;

static NEXT_TEMP_FILE_ID: AtomicU64 = AtomicU64::new(0);

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
    bytes_added: u64,
    bytes_removed: u64,
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
    bytes_added: u64,
    bytes_removed: u64,
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
        "Atomically update an existing UTF-8 file with revision-checked exact text edits."
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Workspace-relative path to an existing UTF-8 text file"
                },
                "expected_revision": {
                    "type": "string",
                    "description": "Revision returned by read_file"
                },
                "edits": {
                    "type": "array",
                    "minItems": 1,
                    "description": "Text replacements to apply. Every old_text is matched against the same original file, and edits must not overlap.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "old_text": {
                                "type": "string",
                                "minLength": 1,
                                "description": "Exact text that must occur exactly once"
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

        replace_file_atomically(
            &resolved,
            &prepared.content,
            metadata.permissions(),
            &prepared.previous_revision,
            &args.path,
        )
        .await?;

        Ok(ApplyPatchOutput {
            path,
            previous_revision: prepared.previous_revision,
            revision: prepared.revision,
            applied_edits: prepared.applied_edits,
            bytes_added: prepared.bytes_added,
            bytes_removed: prepared.bytes_removed,
            message:
                "File was updated atomically. Call read_file again before making further edits."
                    .to_string(),
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
            None,
        ));
    }

    if edits.is_empty() {
        return Err(ToolExecutionError::invalid_args(format!(
            "Cannot update file \"{path}\": edits must contain at least one edit. No changes were made."
        )));
    }

    let mut planned_edits = Vec::with_capacity(edits.len());
    let mut bytes_added = 0_u64;
    let mut bytes_removed = 0_u64;

    for (index, edit) in edits.iter().enumerate() {
        if edit.old_text.is_empty() {
            return Err(ToolExecutionError::invalid_args(format!(
                "Cannot apply edit {} to \"{path}\": old_text must not be empty. No changes were made.",
                index + 1
            )));
        }

        let occurrences = occurrence_positions(original, &edit.old_text);

        match occurrences.as_slice() {
            [] => {
                return Err(ToolExecutionError::invalid_args(format!(
                    "Cannot apply edit {} to \"{path}\": old_text was not found in the original file. No changes were made. Call read_file again and provide more surrounding context.",
                    index + 1
                )));
            }
            [start] => planned_edits.push(PlannedEdit {
                index,
                start: *start,
                end: *start + edit.old_text.len(),
            }),
            _ => {
                return Err(ToolExecutionError::invalid_args(format!(
                    "Cannot apply edit {} to \"{path}\": old_text occurs {} times in the original file and must occur exactly once. No changes were made. Call read_file again and provide more surrounding context.",
                    index + 1,
                    occurrences.len()
                )));
            }
        }

        bytes_added = bytes_added
            .checked_add(edit.new_text.len() as u64)
            .ok_or_else(|| update_size_error(path))?;
        bytes_removed = bytes_removed
            .checked_add(edit.old_text.len() as u64)
            .ok_or_else(|| update_size_error(path))?;
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
        bytes_added,
        bytes_removed,
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

async fn replace_file_atomically(
    target: &Path,
    content: &str,
    permissions: std::fs::Permissions,
    expected_revision: &str,
    display_path: &str,
) -> Result<(), ToolExecutionError> {
    let parent = target.parent().ok_or_else(|| {
        ToolExecutionError::other(format!(
            "Cannot update file \"{display_path}\": parent directory could not be determined."
        ))
    })?;
    let (mut temporary_file, temporary_path) = create_temporary_file(parent, display_path).await?;

    if let Err(error) = write_temporary_file(&mut temporary_file, content).await {
        drop(temporary_file);
        return Err(temporary_file_error(
            display_path,
            "writing the new content",
            error,
            cleanup_temporary_file(&temporary_path).await,
        ));
    }

    if let Err(error) = fs::set_permissions(&temporary_path, permissions).await {
        drop(temporary_file);
        return Err(temporary_file_error(
            display_path,
            "preserving file permissions",
            error,
            cleanup_temporary_file(&temporary_path).await,
        ));
    }

    if let Err(error) = temporary_file.sync_all().await {
        drop(temporary_file);
        return Err(temporary_file_error(
            display_path,
            "synchronizing the new content",
            error,
            cleanup_temporary_file(&temporary_path).await,
        ));
    }

    drop(temporary_file);

    let current_bytes = match fs::read(target).await {
        Ok(content) => content,
        Err(error) => {
            let cleanup_error = cleanup_temporary_file(&temporary_path).await;
            return Err(update_io_error_with_cleanup(
                display_path,
                "checking the revision before commit",
                error,
                cleanup_error,
            ));
        }
    };
    let current_revision = sha256(&current_bytes);

    if current_revision != expected_revision {
        let cleanup_error = cleanup_temporary_file(&temporary_path).await;
        return Err(revision_mismatch_error(
            display_path,
            expected_revision,
            &current_revision,
            cleanup_error,
        ));
    }

    if let Err(error) = fs::rename(&temporary_path, target).await {
        return Err(temporary_file_error(
            display_path,
            "atomically replacing the original file",
            error,
            cleanup_temporary_file(&temporary_path).await,
        ));
    }

    Ok(())
}

async fn create_temporary_file(
    parent: &Path,
    display_path: &str,
) -> Result<(File, PathBuf), ToolExecutionError> {
    for _ in 0..32 {
        let id = NEXT_TEMP_FILE_ID.fetch_add(1, Ordering::Relaxed);
        let temporary_path = parent.join(format!(
            ".rigel-apply-patch-{}-{id}.tmp",
            std::process::id()
        ));

        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)
            .await
        {
            Ok(file) => return Ok((file, temporary_path)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(update_io_error(
                    display_path,
                    "creating a temporary file",
                    error,
                ));
            }
        }
    }

    Err(ToolExecutionError::other(format!(
        "Cannot update file \"{display_path}\": could not allocate a unique temporary file after 32 attempts. No changes were made."
    )))
}

async fn write_temporary_file(file: &mut File, content: &str) -> std::io::Result<()> {
    file.write_all(content.as_bytes()).await?;
    file.flush().await
}

async fn cleanup_temporary_file(path: &Path) -> Option<std::io::Error> {
    match fs::remove_file(path).await {
        Ok(()) => None,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => Some(error),
    }
}

fn update_size_error(path: &str) -> ToolExecutionError {
    ToolExecutionError::other(format!(
        "Cannot update file \"{path}\": edit byte counts exceed the supported size. No changes were made."
    ))
}

fn revision_mismatch_error(
    path: &str,
    expected_revision: &str,
    current_revision: &str,
    cleanup_error: Option<std::io::Error>,
) -> ToolExecutionError {
    let mut message = format!(
        "Cannot update file \"{path}\": expected_revision \"{expected_revision}\" does not match the current revision \"{current_revision}\". No edits were committed. Call read_file again before retrying."
    );

    if let Some(error) = cleanup_error {
        message.push_str(&format!(
            " The temporary file could not be removed: {error}."
        ));
    }

    ToolExecutionError::invalid_args(message)
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

fn update_io_error_with_cleanup(
    path: &str,
    operation: &str,
    error: std::io::Error,
    cleanup_error: Option<std::io::Error>,
) -> ToolExecutionError {
    if let Some(cleanup_error) = cleanup_error {
        return ToolExecutionError::other(format!(
            "Cannot update file \"{path}\" while {operation}: {error}. The temporary file could not be removed: {cleanup_error}."
        ))
        .with_source(error);
    }

    update_io_error(path, operation, error)
}

fn temporary_file_error(
    path: &str,
    operation: &str,
    error: std::io::Error,
    cleanup_error: Option<std::io::Error>,
) -> ToolExecutionError {
    let mut message = format!(
        "Cannot update file \"{path}\" while {operation}: {error}. The original file was not replaced."
    );

    if let Some(cleanup_error) = cleanup_error {
        message.push_str(&format!(
            " The temporary file could not be removed: {cleanup_error}."
        ));
    }

    match error.kind() {
        std::io::ErrorKind::PermissionDenied => {
            ToolExecutionError::permission_denied(message).with_source(error)
        }
        _ => ToolExecutionError::other(message).with_source(error),
    }
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
    fn revision_mismatch_requests_read_file() {
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
                .is_some_and(|message| message.contains("Call read_file again before retrying"))
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
    fn successful_update_reports_revisions_and_byte_counts() {
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
        assert_eq!(prepared.bytes_added, 5);
        assert_eq!(prepared.bytes_removed, 5);
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
}
