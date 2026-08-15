use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use rig::tool::ToolExecutionError;
use tokio::{
    fs::{self, File, OpenOptions},
    io::AsyncWriteExt,
};

use crate::tools::{contracts::error_codes, revision::sha256};

use super::tool_apply_patch::{coded_error, revision_changed_error, update_io_error};

const TEMP_FILE_ATTEMPTS: usize = 8;
static NEXT_TEMP_FILE_ID: AtomicU64 = AtomicU64::new(0);

pub(crate) async fn replace_file(
    target: &Path,
    content: &str,
    expected_revision: &str,
    display_path: &str,
) -> Result<(), ToolExecutionError> {
    let metadata = fs::metadata(target)
        .await
        .map_err(|error| update_io_error(display_path, "reading file metadata", error))?;
    verify_revision(
        target,
        expected_revision,
        display_path,
        "checking the revision before writing",
    )
    .await?;
    let temporary = create_temporary_file(target, display_path).await?;
    let temporary_path =
        write_temporary_file(temporary, content, display_path, metadata.permissions()).await?;
    commit_temporary_file(target, temporary_path, expected_revision, display_path).await
}

async fn verify_revision(
    target: &Path,
    expected_revision: &str,
    display_path: &str,
    operation: &str,
) -> Result<(), ToolExecutionError> {
    let bytes = fs::read(target)
        .await
        .map_err(|error| update_io_error(display_path, operation, error))?;
    let current_revision = sha256(&bytes);
    if current_revision != expected_revision {
        return Err(revision_changed_error(
            display_path,
            expected_revision,
            &current_revision,
        ));
    }
    Ok(())
}

async fn create_temporary_file(
    target: &Path,
    display_path: &str,
) -> Result<(PathBuf, File), ToolExecutionError> {
    for _ in 0..TEMP_FILE_ATTEMPTS {
        let path = next_temporary_path(target)?;
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .await
        {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(update_io_error(
                    display_path,
                    "creating the temporary file",
                    error,
                ));
            }
        }
    }
    Err(coded_error(
        error_codes::IO_ERROR,
        format!("Cannot update file \"{display_path}\": could not create a unique temporary file."),
    ))
}

fn next_temporary_path(target: &Path) -> Result<PathBuf, ToolExecutionError> {
    let parent = target.parent().ok_or_else(|| {
        coded_error(
            error_codes::INVALID_ARGUMENT,
            "Cannot update file: target parent is missing.",
        )
    })?;
    let name = target.file_name().ok_or_else(|| {
        coded_error(
            error_codes::INVALID_ARGUMENT,
            "Cannot update file: target file name is missing.",
        )
    })?;
    let id = NEXT_TEMP_FILE_ID.fetch_add(1, Ordering::Relaxed);
    Ok(parent.join(format!(
        ".{}.rigel-{}-{}.tmp",
        name.to_string_lossy(),
        std::process::id(),
        id
    )))
}

async fn write_temporary_file(
    (path, mut file): (PathBuf, File),
    content: &str,
    display_path: &str,
    permissions: std::fs::Permissions,
) -> Result<PathBuf, ToolExecutionError> {
    if let Err(error) = write_temporary_content(&mut file, content, display_path).await {
        drop(file);
        return Err(cleanup_temporary_file(path, error, display_path).await);
    }
    let permissions_result = fs::set_permissions(&path, permissions)
        .await
        .map_err(|error| update_io_error(display_path, "preserving file permissions", error));
    drop(file);
    match permissions_result {
        Ok(()) => Ok(path),
        Err(error) => Err(cleanup_temporary_file(path, error, display_path).await),
    }
}

async fn write_temporary_content(
    file: &mut File,
    content: &str,
    display_path: &str,
) -> Result<(), ToolExecutionError> {
    file.write_all(content.as_bytes())
        .await
        .map_err(|error| update_io_error(display_path, "writing the temporary file", error))?;
    file.flush()
        .await
        .map_err(|error| update_io_error(display_path, "flushing the temporary file", error))?;
    file.sync_all()
        .await
        .map_err(|error| update_io_error(display_path, "syncing the temporary file", error))
}

async fn commit_temporary_file(
    target: &Path,
    temporary_path: PathBuf,
    expected_revision: &str,
    display_path: &str,
) -> Result<(), ToolExecutionError> {
    if let Err(error) = verify_revision(
        target,
        expected_revision,
        display_path,
        "checking the revision before committing",
    )
    .await
    {
        return Err(cleanup_temporary_file(temporary_path, error, display_path).await);
    }
    match fs::rename(&temporary_path, target).await {
        Ok(()) => Ok(()),
        Err(error) => Err(cleanup_temporary_file(
            temporary_path,
            update_io_error(display_path, "committing the updated file", error),
            display_path,
        )
        .await),
    }
}

async fn cleanup_temporary_file(
    path: PathBuf,
    primary_error: ToolExecutionError,
    display_path: &str,
) -> ToolExecutionError {
    match fs::remove_file(path).await {
        Ok(()) => primary_error,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => primary_error,
        Err(error) => update_io_error(display_path, "cleaning up the temporary file", error),
    }
}
