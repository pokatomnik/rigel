use std::path::PathBuf;

use rig::tool::ToolExecutionError;

use crate::tools::{
    error_codes,
    utils::{
        errors::invalid_argument as invalid_argument_error,
        path::{MAX_PATH_BYTES, validate_relative_path},
        text::is_supported_text,
    },
};

/// Maximum UTF-8 byte length accepted for either exact replacement fragment.
pub(super) const MAX_EDIT_TEXT_BYTES: usize = 64 * 1024;

/// Maximum UTF-8 file size accepted for one edit transaction.
pub(super) const MAX_FILE_BYTES: usize = 1024 * 1024;

/// Validates scalar arguments before any filesystem access occurs.
pub(super) fn validate_arguments(
    path: &str,
    old_text: &str,
    new_text: &str,
) -> Result<(), ToolExecutionError> {
    validate_path(path)?;
    if old_text.is_empty() || old_text == new_text {
        return Err(invalid_argument_error(
            "old_text must be non-empty and different from new_text; reread the file and provide one intended replacement.",
        ));
    }
    if old_text.len() > MAX_EDIT_TEXT_BYTES || new_text.len() > MAX_EDIT_TEXT_BYTES {
        return Err(invalid_argument_error(
            "old_text and new_text must each be at most 65536 bytes; provide a smaller exact replacement.",
        ));
    }
    if new_text.contains('\0') || !is_supported_text(new_text) {
        return Err(unsupported_file_error(path));
    }
    Ok(())
}

/// Validates a relative path lexically before canonicalization.
pub(super) fn validate_path(path: &str) -> Result<PathBuf, ToolExecutionError> {
    validate_relative_path(path, MAX_PATH_BYTES, "edit")
}

/// Returns the byte range of the only exact match, rejecting zero or many matches.
pub(super) fn unique_match(
    text: &str,
    old_text: &str,
) -> Result<(usize, usize), ToolExecutionError> {
    let Some(start) = text.find(old_text) else {
        return Err(no_match_error(old_text));
    };
    if text.rfind(old_text) != Some(start) {
        return Err(ambiguous_match_error(old_text));
    }
    Ok((start, start + old_text.len()))
}

/// Applies a replacement after the caller has proved its range is unique.
pub(super) fn replaced_text(text: &str, start: usize, end: usize, replacement: &str) -> String {
    let mut updated = String::with_capacity(text.len() - (end - start) + replacement.len());
    updated.push_str(&text[..start]);
    updated.push_str(replacement);
    updated.push_str(&text[end..]);
    updated
}

/// Rejects a result that would exceed the bounded edit-file input/output budget.
pub(super) fn validate_result_size(size: usize) -> Result<(), ToolExecutionError> {
    if size > MAX_FILE_BYTES {
        return Err(invalid_argument_error(
            "the edited file would exceed the 1048576-byte server limit; use a smaller targeted edit.",
        ));
    }
    Ok(())
}

/// Reports binary, control-heavy, directory, and other unsupported targets.
pub(super) fn unsupported_file_error(path: &str) -> ToolExecutionError {
    ToolExecutionError::other(format!(
        "Cannot edit file \"{path}\": only an existing regular UTF-8 text file is supported. Reread a text file and retry."
    ))
    .with_code(error_codes::UNSUPPORTED_FILE)
}

/// Reports a source file that exceeds the bounded edit transaction size.
pub(super) fn file_too_large_error(path: &str) -> ToolExecutionError {
    invalid_argument_error(format!(
        "Cannot edit file \"{path}\": it exceeds the 1048576-byte server limit. Use a smaller targeted edit."
    ))
}

fn no_match_error(old_text: &str) -> ToolExecutionError {
    invalid_argument_error(format!(
        "NO_MATCH: old_text does not occur in the current file. Reread the file and provide exact current old_text: {old_text:?}."
    ))
    .with_code(error_codes::NO_MATCH)
}

fn ambiguous_match_error(old_text: &str) -> ToolExecutionError {
    invalid_argument_error(format!(
        "AMBIGUOUS_MATCH: old_text occurs more than once. Reread the file and include more unique context: {old_text:?}."
    ))
    .with_code(error_codes::AMBIGUOUS_MATCH)
}
