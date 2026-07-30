use std::{
    env,
    path::{Component, Path, PathBuf},
};

use futures::future::BoxFuture;
use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::{Deserialize, Serialize};
use tokio::fs;

#[derive(Deserialize)]
pub(crate) struct MovePathArgs {
    from: String,
    to: String,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct MovePathOutput {
    from: String,
    to: String,
    item_type: MovedItemType,
    action: MoveAction,
    destination_directories_created: bool,
    replaced_files: usize,
    message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum MovedItemType {
    File,
    Directory,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum MoveAction {
    Moved,
    Merged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EntryKind {
    File,
    Directory,
    Symlink,
    Other,
}

struct InspectedPath {
    path: PathBuf,
    kind: Option<EntryKind>,
}

enum MergeOperation {
    MoveEntry {
        from: PathBuf,
        to: PathBuf,
        replace: bool,
    },
    MoveDirectory {
        from: PathBuf,
        to: PathBuf,
    },
    RemoveDirectory {
        path: PathBuf,
    },
}

#[derive(Default)]
struct MergePlan {
    operations: Vec<MergeOperation>,
    replaced_files: usize,
}

pub(crate) struct MovePath {
    root: PathBuf,
}

impl MovePath {
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

    fn normalize_relative_path(
        path: &str,
        argument: &str,
        allow_root: bool,
    ) -> Result<PathBuf, ToolExecutionError> {
        if path.is_empty() {
            return Err(ToolExecutionError::invalid_args(format!(
                "Cannot move path: \"{argument}\" must not be empty."
            )));
        }

        let mut normalized = PathBuf::new();

        for component in Path::new(path).components() {
            match component {
                Component::Normal(component) => normalized.push(component),
                Component::ParentDir if !normalized.pop() => {
                    return Err(outside_project_error(path, argument));
                }
                Component::ParentDir | Component::CurDir => {}
                Component::RootDir | Component::Prefix(_) => {
                    return Err(ToolExecutionError::invalid_args(format!(
                        "Cannot move path: \"{argument}\" value \"{path}\" must be relative to the project root."
                    )));
                }
            }
        }

        if !allow_root && normalized.as_os_str().is_empty() {
            return Err(ToolExecutionError::refused(format!(
                "Cannot move path \"{path}\": moving the project root is not allowed."
            )));
        }

        Ok(normalized)
    }

    async fn inspect_path(
        &self,
        relative: &Path,
        original: &str,
        argument: &str,
    ) -> Result<InspectedPath, ToolExecutionError> {
        let lexical_path = self.root.join(relative);

        match fs::symlink_metadata(&lexical_path).await {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                let path = self
                    .resolve_entry_location(&lexical_path, original, argument)
                    .await?;
                Ok(InspectedPath {
                    path,
                    kind: Some(EntryKind::Symlink),
                })
            }
            Ok(metadata) => {
                let resolved = fs::canonicalize(&lexical_path)
                    .await
                    .map_err(|error| inspect_error(original, argument, error))?;

                if !resolved.starts_with(&self.root) {
                    return Err(outside_project_error(original, argument));
                }

                Ok(InspectedPath {
                    path: resolved,
                    kind: Some(entry_kind(&metadata)),
                })
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.inspect_missing_path(lexical_path, original, argument)
                    .await
            }
            Err(error) => Err(inspect_error(original, argument, error)),
        }
    }

    async fn inspect_missing_path(
        &self,
        lexical_path: PathBuf,
        original: &str,
        argument: &str,
    ) -> Result<InspectedPath, ToolExecutionError> {
        let mut ancestor = lexical_path.clone();

        loop {
            match fs::canonicalize(&ancestor).await {
                Ok(resolved_ancestor) => {
                    if !resolved_ancestor.starts_with(&self.root) {
                        return Err(outside_project_error(original, argument));
                    }

                    let metadata = fs::metadata(&resolved_ancestor)
                        .await
                        .map_err(|error| inspect_error(original, argument, error))?;

                    if !metadata.is_dir() {
                        return Err(ToolExecutionError::invalid_args(format!(
                            "Cannot move path: \"{argument}\" value \"{original}\" contains a component that is not a directory."
                        )));
                    }

                    let suffix = lexical_path.strip_prefix(&ancestor).map_err(|_| {
                        ToolExecutionError::other(format!(
                            "Cannot resolve \"{argument}\" path \"{original}\" inside the project."
                        ))
                    })?;

                    return Ok(InspectedPath {
                        path: resolved_ancestor.join(suffix),
                        kind: None,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    if !ancestor.pop() {
                        return Err(ToolExecutionError::other(format!(
                            "Cannot inspect \"{argument}\" path \"{original}\": no accessible parent directory was found."
                        ))
                        .with_source(error));
                    }
                }
                Err(error) => return Err(inspect_error(original, argument, error)),
            }
        }
    }

    async fn inspect_absolute_destination(
        &self,
        path: PathBuf,
        display_path: &str,
    ) -> Result<InspectedPath, ToolExecutionError> {
        match fs::symlink_metadata(&path).await {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                let path = self
                    .resolve_entry_location(&path, display_path, "to")
                    .await?;
                Ok(InspectedPath {
                    path,
                    kind: Some(EntryKind::Symlink),
                })
            }
            Ok(metadata) => {
                let resolved = fs::canonicalize(&path)
                    .await
                    .map_err(|error| inspect_error(display_path, "to", error))?;

                if !resolved.starts_with(&self.root) {
                    return Err(outside_project_error(display_path, "to"));
                }

                Ok(InspectedPath {
                    path: resolved,
                    kind: Some(entry_kind(&metadata)),
                })
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(InspectedPath { path, kind: None })
            }
            Err(error) => Err(inspect_error(display_path, "to", error)),
        }
    }

    async fn resolve_entry_location(
        &self,
        path: &Path,
        original: &str,
        argument: &str,
    ) -> Result<PathBuf, ToolExecutionError> {
        let parent = path.parent().ok_or_else(|| {
            ToolExecutionError::invalid_args(format!(
                "Cannot inspect \"{argument}\" path \"{original}\": parent path could not be determined."
            ))
        })?;
        let resolved_parent = fs::canonicalize(parent)
            .await
            .map_err(|error| inspect_error(original, argument, error))?;

        if !resolved_parent.starts_with(&self.root) {
            return Err(outside_project_error(original, argument));
        }

        let name = path.file_name().ok_or_else(|| {
            ToolExecutionError::invalid_args(format!(
                "Cannot inspect \"{argument}\" path \"{original}\": entry name could not be determined."
            ))
        })?;

        Ok(resolved_parent.join(name))
    }

    async fn ensure_parent_directory(
        &self,
        target: &Path,
        from: &str,
        to: &str,
    ) -> Result<(PathBuf, bool), ToolExecutionError> {
        let parent = target.parent().ok_or_else(|| {
            ToolExecutionError::invalid_args(format!(
                "Cannot move \"{from}\" to \"{to}\": destination parent could not be determined."
            ))
        })?;
        let parent_existed = fs::metadata(parent)
            .await
            .is_ok_and(|metadata| metadata.is_dir());

        fs::create_dir_all(parent)
            .await
            .map_err(|error| move_error(from, to, "creating destination directories", error))?;

        let resolved_parent = fs::canonicalize(parent)
            .await
            .map_err(|error| move_error(from, to, "resolving destination directory", error))?;

        if !resolved_parent.starts_with(&self.root) {
            return Err(outside_project_error(to, "to"));
        }

        let name = target.file_name().ok_or_else(|| {
            ToolExecutionError::invalid_args(format!(
                "Cannot move \"{from}\" to \"{to}\": destination does not name a path."
            ))
        })?;

        Ok((resolved_parent.join(name), !parent_existed))
    }

    fn build_merge_plan<'a>(
        &'a self,
        source: &'a Path,
        destination: &'a Path,
        from: &'a str,
        to: &'a str,
        plan: &'a mut MergePlan,
    ) -> BoxFuture<'a, Result<(), ToolExecutionError>> {
        Box::pin(async move {
            let mut entries = fs::read_dir(source)
                .await
                .map_err(|error| move_error(from, to, "reading the source directory", error))?;

            while let Some(entry) = entries
                .next_entry()
                .await
                .map_err(|error| move_error(from, to, "reading a source entry", error))?
            {
                let source_path = entry.path();
                let destination_path = destination.join(entry.file_name());
                let source_metadata = fs::symlink_metadata(&source_path)
                    .await
                    .map_err(|error| move_error(from, to, "inspecting a source entry", error))?;
                let source_kind = entry_kind(&source_metadata);
                let destination_state = self
                    .inspect_absolute_destination(destination_path.clone(), to)
                    .await?;

                match (source_kind, destination_state.kind) {
                    (EntryKind::Directory, Some(EntryKind::Directory)) => {
                        self.build_merge_plan(
                            &source_path,
                            &destination_state.path,
                            from,
                            to,
                            plan,
                        )
                        .await?;
                    }
                    (EntryKind::Directory, None) => {
                        plan.operations.push(MergeOperation::MoveDirectory {
                            from: source_path,
                            to: destination_state.path,
                        });
                    }
                    (EntryKind::Directory, Some(_)) => {
                        return Err(merge_conflict_error(
                            &self.root,
                            from,
                            to,
                            &source_path,
                            "source entry is a directory but destination entry is not",
                        ));
                    }
                    (EntryKind::File | EntryKind::Symlink, Some(EntryKind::Directory)) => {
                        return Err(merge_conflict_error(
                            &self.root,
                            from,
                            to,
                            &source_path,
                            "source entry is a file but destination entry is a directory",
                        ));
                    }
                    (
                        EntryKind::File | EntryKind::Symlink,
                        Some(EntryKind::File | EntryKind::Symlink),
                    ) => {
                        plan.replaced_files += 1;
                        plan.operations.push(MergeOperation::MoveEntry {
                            from: source_path,
                            to: destination_state.path,
                            replace: true,
                        });
                    }
                    (EntryKind::File | EntryKind::Symlink, None) => {
                        plan.operations.push(MergeOperation::MoveEntry {
                            from: source_path,
                            to: destination_state.path,
                            replace: false,
                        });
                    }
                    (EntryKind::File | EntryKind::Symlink, Some(EntryKind::Other))
                    | (EntryKind::Other, _) => {
                        return Err(merge_conflict_error(
                            &self.root,
                            from,
                            to,
                            &source_path,
                            "unsupported filesystem entry type",
                        ));
                    }
                }
            }

            plan.operations.push(MergeOperation::RemoveDirectory {
                path: source.to_path_buf(),
            });

            Ok(())
        })
    }

    async fn execute_merge_plan(
        &self,
        plan: MergePlan,
        from: &str,
        to: &str,
    ) -> Result<usize, ToolExecutionError> {
        let mut completed_moves = 0_usize;

        for operation in plan.operations {
            match operation {
                MergeOperation::MoveEntry {
                    from: source,
                    to: destination,
                    replace,
                } => {
                    if replace {
                        fs::remove_file(&destination).await.map_err(|error| {
                            partial_move_error(
                                from,
                                to,
                                completed_moves,
                                "removing an existing destination file",
                                error,
                            )
                        })?;
                    }

                    fs::rename(&source, &destination).await.map_err(|error| {
                        let stage = if replace {
                            "moving a source file after its destination was removed"
                        } else {
                            "moving a source file"
                        };
                        partial_move_error(from, to, completed_moves, stage, error)
                    })?;
                    completed_moves += 1;
                }
                MergeOperation::MoveDirectory {
                    from: source,
                    to: destination,
                } => {
                    fs::rename(&source, &destination).await.map_err(|error| {
                        partial_move_error(
                            from,
                            to,
                            completed_moves,
                            "moving a source subdirectory",
                            error,
                        )
                    })?;
                    completed_moves += 1;
                }
                MergeOperation::RemoveDirectory { path } => {
                    fs::remove_dir(&path).await.map_err(|error| {
                        partial_move_error(
                            from,
                            to,
                            completed_moves,
                            "removing an emptied source directory",
                            error,
                        )
                    })?;
                }
            }
        }

        Ok(completed_moves)
    }

    async fn move_file(
        &self,
        source: PathBuf,
        destination: InspectedPath,
        from: &str,
        to: &str,
    ) -> Result<MovePathOutput, ToolExecutionError> {
        let source_name = source.file_name().ok_or_else(|| {
            ToolExecutionError::invalid_args(format!(
                "Cannot move file \"{from}\": source file name could not be determined."
            ))
        })?;
        let (destination, display_to) = if destination.kind == Some(EntryKind::Directory) {
            let display_to = Path::new(to)
                .join(source_name)
                .to_string_lossy()
                .into_owned();
            (
                self.inspect_absolute_destination(destination.path.join(source_name), &display_to)
                    .await?,
                display_to,
            )
        } else {
            (destination, to.to_string())
        };

        if destination.path == source {
            return Err(same_path_error(from, &display_to));
        }

        let (final_destination, directories_created) = if destination.kind.is_none() {
            self.ensure_parent_directory(&destination.path, from, &display_to)
                .await?
        } else {
            (destination.path.clone(), false)
        };

        let replaced_file = match destination.kind {
            Some(EntryKind::File | EntryKind::Symlink) => {
                fs::remove_file(&final_destination).await.map_err(|error| {
                    move_error(
                        from,
                        &display_to,
                        "removing the existing destination file",
                        error,
                    )
                })?;
                true
            }
            Some(EntryKind::Directory) => {
                return Err(ToolExecutionError::invalid_args(format!(
                    "Cannot move file \"{from}\" to \"{display_to}\": destination already exists as a directory."
                )));
            }
            Some(EntryKind::Other) => {
                return Err(ToolExecutionError::invalid_args(format!(
                    "Cannot move file \"{from}\" to \"{display_to}\": destination has an unsupported filesystem type."
                )));
            }
            None => false,
        };

        fs::rename(&source, &final_destination)
            .await
            .map_err(|error| {
                let stage = if replaced_file {
                    "moving the source file after the existing destination file was removed"
                } else {
                    "moving the source file"
                };
                move_error(from, &display_to, stage, error)
            })?;

        Ok(move_result(
            from,
            &display_to,
            MovedItemType::File,
            MoveAction::Moved,
            directories_created,
            usize::from(replaced_file),
        ))
    }

    async fn move_directory(
        &self,
        source: PathBuf,
        destination: InspectedPath,
        from: &str,
        to: &str,
    ) -> Result<MovePathOutput, ToolExecutionError> {
        if destination.path == source || destination.path.starts_with(&source) {
            return Err(ToolExecutionError::invalid_args(format!(
                "Cannot move directory \"{from}\" to \"{to}\": destination is the source directory or is inside it."
            )));
        }

        match destination.kind {
            None => {
                let (final_destination, directories_created) = self
                    .ensure_parent_directory(&destination.path, from, to)
                    .await?;

                fs::rename(&source, &final_destination)
                    .await
                    .map_err(|error| move_error(from, to, "moving the source directory", error))?;

                Ok(move_result(
                    from,
                    to,
                    MovedItemType::Directory,
                    MoveAction::Moved,
                    directories_created,
                    0,
                ))
            }
            Some(EntryKind::Directory) => {
                let mut plan = MergePlan::default();
                self.build_merge_plan(&source, &destination.path, from, to, &mut plan)
                    .await?;
                let replaced_files = plan.replaced_files;
                self.execute_merge_plan(plan, from, to).await?;

                Ok(move_result(
                    from,
                    to,
                    MovedItemType::Directory,
                    MoveAction::Merged,
                    false,
                    replaced_files,
                ))
            }
            Some(EntryKind::File | EntryKind::Symlink | EntryKind::Other) => {
                Err(ToolExecutionError::invalid_args(format!(
                    "Cannot move directory \"{from}\" to \"{to}\": destination exists and is not a directory."
                )))
            }
        }
    }
}

impl Tool for MovePath {
    const NAME: &'static str = "move_path";
    type Args = MovePathArgs;
    type Output = MovePathOutput;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Rename or move files and directories inside the project.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "from": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Source file or directory path relative to the project root."
                },
                "to": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Destination path relative to the project root. Missing destination directories are created."
                }
            },
            "required": ["from", "to"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let source_relative = Self::normalize_relative_path(&args.from, "from", false)?;
        let destination_relative = Self::normalize_relative_path(&args.to, "to", true)?;
        let source = self
            .inspect_path(&source_relative, &args.from, "from")
            .await?;
        let destination = self
            .inspect_path(&destination_relative, &args.to, "to")
            .await?;

        match source.kind {
            None => Err(ToolExecutionError::not_found(format!(
                "Cannot move \"{}\": source path does not exist.",
                args.from
            ))),
            Some(EntryKind::File) => {
                self.move_file(source.path, destination, &args.from, &args.to)
                    .await
            }
            Some(EntryKind::Directory) => {
                self.move_directory(source.path, destination, &args.from, &args.to)
                    .await
            }
            Some(EntryKind::Symlink) => Err(ToolExecutionError::invalid_args(format!(
                "Cannot move \"{}\": source path is a symbolic link, not a file or directory.",
                args.from
            ))),
            Some(EntryKind::Other) => Err(ToolExecutionError::invalid_args(format!(
                "Cannot move \"{}\": source has an unsupported filesystem type.",
                args.from
            ))),
        }
    }
}

fn entry_kind(metadata: &std::fs::Metadata) -> EntryKind {
    if metadata.is_dir() {
        EntryKind::Directory
    } else if metadata.is_file() {
        EntryKind::File
    } else if metadata.file_type().is_symlink() {
        EntryKind::Symlink
    } else {
        EntryKind::Other
    }
}

fn move_result(
    from: &str,
    to: &str,
    item_type: MovedItemType,
    action: MoveAction,
    destination_directories_created: bool,
    replaced_files: usize,
) -> MovePathOutput {
    let item = match item_type {
        MovedItemType::File => "File",
        MovedItemType::Directory => "Directory",
    };
    let mut message = match action {
        MoveAction::Moved => format!("{item} \"{from}\" was moved to \"{to}\"."),
        MoveAction::Merged => format!(
            "Directory \"{from}\" was merged into existing directory \"{to}\"; all source contents were moved and the source directory was removed."
        ),
    };

    if destination_directories_created {
        message.push_str(" Missing destination directories were created.");
    }

    if replaced_files == 1 {
        message.push_str(" One existing destination file was replaced.");
    } else if replaced_files > 1 {
        message.push_str(&format!(
            " {replaced_files} existing destination files were replaced."
        ));
    }

    MovePathOutput {
        from: from.to_string(),
        to: to.to_string(),
        item_type,
        action,
        destination_directories_created,
        replaced_files,
        message,
    }
}

fn outside_project_error(path: &str, argument: &str) -> ToolExecutionError {
    ToolExecutionError::refused(format!(
        "Cannot move path: \"{argument}\" value \"{path}\" resolves outside the project root."
    ))
}

fn same_path_error(from: &str, to: &str) -> ToolExecutionError {
    ToolExecutionError::invalid_args(format!(
        "Cannot move \"{from}\" to \"{to}\": source and destination are the same path."
    ))
}

fn merge_conflict_error(
    root: &Path,
    from: &str,
    to: &str,
    source_entry: &Path,
    reason: &str,
) -> ToolExecutionError {
    let source_entry = source_entry.strip_prefix(root).unwrap_or(source_entry);

    ToolExecutionError::invalid_args(format!(
        "Cannot merge directory \"{from}\" into \"{to}\": conflict at source entry \"{}\" ({reason}). No files were moved.",
        source_entry.display()
    ))
}

fn inspect_error(path: &str, argument: &str, error: std::io::Error) -> ToolExecutionError {
    let message = match error.kind() {
        std::io::ErrorKind::PermissionDenied => {
            format!("Cannot inspect \"{argument}\" path \"{path}\": permission denied.")
        }
        std::io::ErrorKind::NotADirectory => {
            format!(
                "Cannot inspect \"{argument}\" path \"{path}\": a path component is not a directory."
            )
        }
        _ => format!("Cannot inspect \"{argument}\" path \"{path}\": {error}"),
    };

    match error.kind() {
        std::io::ErrorKind::PermissionDenied => ToolExecutionError::permission_denied(message),
        std::io::ErrorKind::NotADirectory => ToolExecutionError::invalid_args(message),
        _ => ToolExecutionError::other(message),
    }
    .with_source(error)
}

fn move_error(from: &str, to: &str, stage: &str, error: std::io::Error) -> ToolExecutionError {
    let message = match error.kind() {
        std::io::ErrorKind::PermissionDenied => {
            format!("Cannot move \"{from}\" to \"{to}\" while {stage}: permission denied.")
        }
        std::io::ErrorKind::NotFound => {
            format!(
                "Cannot move \"{from}\" to \"{to}\" while {stage}: a source or destination path no longer exists."
            )
        }
        std::io::ErrorKind::AlreadyExists => {
            format!(
                "Cannot move \"{from}\" to \"{to}\" while {stage}: destination changed and now already exists."
            )
        }
        std::io::ErrorKind::CrossesDevices => {
            format!(
                "Cannot move \"{from}\" to \"{to}\" while {stage}: source and destination are on different filesystems."
            )
        }
        _ => format!("Cannot move \"{from}\" to \"{to}\" while {stage}: {error}"),
    };

    match error.kind() {
        std::io::ErrorKind::PermissionDenied => ToolExecutionError::permission_denied(message),
        std::io::ErrorKind::NotFound => ToolExecutionError::not_found(message),
        std::io::ErrorKind::AlreadyExists => ToolExecutionError::invalid_args(message),
        _ => ToolExecutionError::other(message),
    }
    .with_source(error)
}

fn partial_move_error(
    from: &str,
    to: &str,
    completed_moves: usize,
    stage: &str,
    error: std::io::Error,
) -> ToolExecutionError {
    ToolExecutionError::other(format!(
        "Move from \"{from}\" to \"{to}\" was partially completed after {completed_moves} entries while {stage}: {error}. Inspect both paths before retrying."
    ))
    .with_source(error)
}

#[cfg(test)]
mod tests {
    use rig::tool::ToolErrorKind;

    use super::*;

    #[test]
    fn file_move_result_reports_created_directories_and_replacement() {
        let result = move_result(
            "old.txt",
            "archive/new.txt",
            MovedItemType::File,
            MoveAction::Moved,
            true,
            1,
        );

        assert_eq!(result.item_type, MovedItemType::File);
        assert!(result.destination_directories_created);
        assert_eq!(result.replaced_files, 1);
        assert_eq!(
            result.message,
            "File \"old.txt\" was moved to \"archive/new.txt\". Missing destination directories were created. One existing destination file was replaced."
        );
    }

    #[test]
    fn directory_merge_result_is_explicit() {
        let result = move_result(
            "generated",
            "src",
            MovedItemType::Directory,
            MoveAction::Merged,
            false,
            2,
        );

        assert_eq!(result.action, MoveAction::Merged);
        assert_eq!(result.replaced_files, 2);
        assert_eq!(
            result.message,
            "Directory \"generated\" was merged into existing directory \"src\"; all source contents were moved and the source directory was removed. 2 existing destination files were replaced."
        );
    }

    #[test]
    fn source_project_root_is_refused() {
        let error = MovePath::normalize_relative_path(".", "from", false)
            .expect_err("project root should fail");

        assert!(error.is_refusal());
        assert_eq!(
            error.model_feedback(),
            Some("Cannot move path \".\": moving the project root is not allowed.")
        );
    }

    #[test]
    fn destination_project_root_is_allowed() {
        MovePath::normalize_relative_path(".", "to", true)
            .expect("project root should be a valid destination");
    }

    #[test]
    fn outside_destination_is_refused() {
        let error = MovePath::normalize_relative_path("../outside", "to", true)
            .expect_err("outside path should fail");

        assert!(error.is_refusal());
        assert_eq!(
            error.model_feedback(),
            Some(
                "Cannot move path: \"to\" value \"../outside\" resolves outside the project root."
            )
        );
    }

    #[test]
    fn same_path_error_is_clear_and_model_visible() {
        let error = same_path_error("file.txt", "file.txt");

        assert_eq!(error.kind(), ToolErrorKind::InvalidArgs);
        assert_eq!(
            error.model_feedback(),
            Some(
                "Cannot move \"file.txt\" to \"file.txt\": source and destination are the same path."
            )
        );
    }
}
