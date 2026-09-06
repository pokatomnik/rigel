use std::{collections::BTreeSet, path::PathBuf};

use crate::tools::action::Action;

use super::{glob_files::GlobFilesOutput, pattern::GlobPattern};

const MAX_MATCHES_LIMIT: usize = 100;
const MAX_PENDING_DIRECTORIES: usize = 10_000;
const TRUNCATION_MESSAGE: &str = "Glob results are truncated by server limits. match_count is the number of returned files; use a more specific pattern to continue.";

#[cfg(test)]
pub(super) const MAX_MATCHES: usize = MAX_MATCHES_LIMIT;

/// Per-call traversal state that owns result ordering, deduplication, and limits.
pub(super) struct GlobScan {
    pattern: GlobPattern,
    files: BTreeSet<String>,
    pending: Vec<PathBuf>,
    directories: usize,
    truncated: bool,
    stop_requested: bool,
}

impl GlobScan {
    /// Creates bounded scan state for one parsed pattern.
    pub(super) fn new(pattern: GlobPattern) -> Self {
        Self {
            pattern,
            files: BTreeSet::new(),
            pending: Vec::new(),
            directories: 0,
            truncated: false,
            stop_requested: false,
        }
    }

    /// Adds the canonical workspace root as the first directory to inspect.
    pub(super) fn add_root(&mut self, root: PathBuf) {
        self.pending.push(root);
    }

    /// Takes the next deterministic directory selected by the traversal.
    pub(super) fn next_directory(&mut self) -> Option<PathBuf> {
        self.pending.pop()
    }

    /// Reports whether the bounded directory traversal has reached its limit.
    pub(super) fn directories_reached(&self, limit: usize) -> bool {
        self.directories >= limit
    }

    /// Accounts for one directory being inspected.
    pub(super) fn count_directory(&mut self) {
        self.directories += 1;
    }

    /// Queues a canonical directory or marks the result incomplete at the queue cap.
    pub(super) fn queue_directory(&mut self, path: PathBuf) {
        if self.pending.len() >= MAX_PENDING_DIRECTORIES {
            self.mark_truncated();
            self.stop();
        } else {
            self.pending.push(path);
        }
    }

    /// Sorts pending directories in reverse order so popping visits ascending paths.
    pub(super) fn sort_pending(&mut self) {
        self.pending
            .sort_by(|left, right| right.to_string_lossy().cmp(&left.to_string_lossy()));
    }

    /// Records a matching path once and stops after the bounded result cap is exceeded.
    pub(super) fn record_if_match(&mut self, path: String) {
        if !self.pattern.matches(&path) || self.files.contains(&path) {
            return;
        }
        if self.files.len() >= MAX_MATCHES_LIMIT {
            self.mark_truncated();
            self.stop();
            return;
        }
        self.files.insert(path);
    }

    /// Finalizes sorted unique paths and preserves successful no-match semantics.
    pub(super) fn output(self) -> GlobFilesOutput {
        let files = self.files.into_iter().collect::<Vec<_>>();
        let truncated = self.truncated;
        GlobFilesOutput {
            action: Action::Matched,
            ok: true,
            pattern: self.pattern.normalized().to_string(),
            match_count: files.len(),
            files,
            truncated,
            message: truncated.then(|| TRUNCATION_MESSAGE.to_string()),
        }
    }

    /// Marks the result incomplete because a server-side scan or result limit was reached.
    pub(super) fn mark_truncated(&mut self) {
        self.truncated = true;
    }

    /// Prevents further traversal after a result or scan boundary is reached.
    pub(super) fn stop(&mut self) {
        self.stop_requested = true;
    }

    /// Reports whether the current scan must stop before another entry is inspected.
    pub(super) fn should_stop(&self) -> bool {
        self.stop_requested
    }
}
