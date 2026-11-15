use std::path::{Path, PathBuf};

use crate::shared::hash::sha256;

const RIGEL_DIRECTORY: &str = ".rigel";
const ACTIVE_FILE_NAME: &str = "chat.jsonl";
const LOCK_FILE_NAME: &str = "chat.lock";
const ARCHIVE_PREFIX: &str = "chat.";
const ARCHIVE_SUFFIX: &str = ".jsonl";

/// Resolved filesystem locations for one chat history family.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HistoryPaths {
    directory: PathBuf,
}

impl HistoryPaths {
    /// Resolves the history directory from the startup working directory.
    pub(crate) fn from_current_dir() -> anyhow::Result<Self> {
        let current_dir = std::env::current_dir()?;
        let absolute = std::fs::canonicalize(current_dir.as_path()).unwrap_or(current_dir);
        let home = std::env::home_dir()
            .ok_or_else(|| anyhow::anyhow!("failed to determine the user home directory"))?;
        let identity = sha256(absolute.to_string_lossy().as_bytes());
        Ok(Self {
            directory: home.join(RIGEL_DIRECTORY).join(identity),
        })
    }

    /// Creates explicit paths for tests and for callers that already own a directory.
    #[cfg(test)]
    pub(crate) fn from_directory(directory: PathBuf) -> Self {
        Self { directory }
    }

    /// Returns the directory that owns every segment and the lock file.
    pub(crate) fn directory(&self) -> &Path {
        self.directory.as_path()
    }

    /// Returns the active append-only segment.
    pub(crate) fn active(&self) -> PathBuf {
        self.directory.join(ACTIVE_FILE_NAME)
    }

    /// Returns the sealed archive segment with the given numeric index.
    pub(crate) fn archive(&self, index: usize) -> PathBuf {
        self.directory
            .join(format!("{ARCHIVE_PREFIX}{index}{ARCHIVE_SUFFIX}"))
    }

    /// Returns the interprocess lock file path.
    pub(crate) fn lock(&self) -> PathBuf {
        self.directory.join(LOCK_FILE_NAME)
    }

    /// Parses the numeric index from an archive file name.
    pub(crate) fn archive_index(file_name: &str) -> Option<usize> {
        let index = file_name
            .strip_prefix(ARCHIVE_PREFIX)?
            .strip_suffix(ARCHIVE_SUFFIX)?;
        index.parse().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::HistoryPaths;

    #[test]
    fn archive_names_parse_to_numeric_indices() {
        assert_eq!(HistoryPaths::archive_index("chat.0.jsonl"), Some(0));
        assert_eq!(HistoryPaths::archive_index("chat.12.jsonl"), Some(12));
    }

    #[test]
    fn non_archive_names_are_ignored() {
        for name in ["chat.jsonl", "chat.lock", "chat.a.jsonl", "other.0.jsonl"] {
            assert_eq!(HistoryPaths::archive_index(name), None, "{name}");
        }
    }

    #[test]
    fn archive_paths_use_the_numeric_suffix() {
        let paths = HistoryPaths::from_directory("dir".into());

        assert_eq!(paths.active(), std::path::Path::new("dir/chat.jsonl"));
        assert_eq!(paths.archive(3), std::path::Path::new("dir/chat.3.jsonl"));
        assert_eq!(paths.lock(), std::path::Path::new("dir/chat.lock"));
    }
}
