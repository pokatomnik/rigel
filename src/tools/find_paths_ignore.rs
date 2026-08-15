use std::{path::Path, sync::Arc};

use ignore::{
    Match,
    gitignore::{Gitignore, GitignoreBuilder},
};
use rig::tool::{ToolErrorKind, ToolExecutionError};
use tokio::fs;

use crate::tools::contracts::error_codes;

use super::find_paths::{
    coded_error, find_io_error, outside_current_directory_error, path_for_output,
};

pub(crate) const IGNORE_FILE_NAMES: [&str; 2] = [".gitignore", ".ignore"];

#[derive(Clone, Default)]
pub(crate) struct IgnoreStack {
    current: Option<Arc<IgnoreLayer>>,
}

struct IgnoreLayer {
    parent: IgnoreStack,
    gitignore: Gitignore,
    ignore: Gitignore,
}

pub(crate) async fn load_directory_ignore_files(
    current_directory_root: &Path,
    physical_directory: &Path,
    logical_directory: &Path,
    stack: IgnoreStack,
) -> Result<IgnoreStack, ToolExecutionError> {
    let gitignore = load_ignore_file(
        current_directory_root,
        physical_directory,
        logical_directory,
        IGNORE_FILE_NAMES[0],
    )
    .await?;
    let ignore = load_ignore_file(
        current_directory_root,
        physical_directory,
        logical_directory,
        IGNORE_FILE_NAMES[1],
    )
    .await?;
    Ok(stack.push(gitignore, ignore))
}

async fn load_ignore_file(
    current_directory_root: &Path,
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
                "resolving an ignore file",
                error,
            ));
        }
    };
    if !resolved_path.starts_with(current_directory_root) {
        return Err(outside_current_directory_error(&path_for_output(
            &logical_path,
        )));
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

pub(crate) fn compile_ignore_content(
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
                    error_codes::INVALID_ARGUMENT,
                    format!(
                        "Cannot parse ignore rule at \"{}\":{}: {error}",
                        path_for_output(source),
                        index + 1
                    ),
                )
            })?;
    }
    builder.build().map_err(|error| {
        coded_error(
            ToolErrorKind::InvalidArgs,
            error_codes::INVALID_ARGUMENT,
            format!(
                "Cannot compile ignore rules from \"{}\": {error}",
                path_for_output(source)
            ),
        )
    })
}

impl IgnoreStack {
    pub(crate) fn push(self, gitignore: Gitignore, ignore: Gitignore) -> Self {
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

    pub(crate) fn is_ignored(&self, path: &Path, is_directory: bool) -> bool {
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
