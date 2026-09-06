use std::path::PathBuf;

use serde::Serialize;

use crate::tools::{action::Action, utils::text::utf8_prefix};

#[cfg(test)]
/// Test-visible copy of the production result cap.
pub(crate) const MAX_MATCHES: usize = 100;
const MAX_MATCHES_LIMIT: usize = 100;
const MAX_MATCH_CONTENT_BYTES: usize = 1024;
const MAX_MATCH_OUTPUT_BYTES: usize = 32 * 1024;
const MAX_WARNINGS: usize = 8;
const MAX_WARNING_BYTES: usize = 240;
const TRUNCATION_MESSAGE: &str = "Search results are truncated by server limits. match_count is the number of returned matches; search a more specific path or pattern to continue.";

/// Stable model-facing result for a successful bounded content search.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct SearchFilesOutput {
    action: Action,
    ok: bool,
    pattern: String,
    path: String,
    matches: Vec<SearchMatch>,
    match_count: usize,
    truncated: bool,
    next_offset: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    warnings: Option<Vec<String>>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
struct SearchMatch {
    path: String,
    line: usize,
    content: String,
}

/// Per-call search state that owns result, scan, truncation, and warning limits.
pub(crate) struct SearchContext {
    root: PathBuf,
    pattern: String,
    matches: Vec<SearchMatch>,
    scanned_bytes: usize,
    directories: usize,
    output_bytes: usize,
    truncated: bool,
    stop_requested: bool,
    warnings: Vec<String>,
    warnings_truncated: bool,
}

impl SearchContext {
    /// Creates a search state; the pattern remains literal and case-sensitive.
    pub(crate) fn new(root: PathBuf, pattern: &str) -> Self {
        Self {
            root,
            pattern: pattern.to_string(),
            matches: Vec::new(),
            scanned_bytes: 0,
            directories: 0,
            output_bytes: 0,
            truncated: false,
            stop_requested: false,
            warnings: Vec::new(),
            warnings_truncated: false,
        }
    }

    /// Returns the canonical workspace path whose children may be scanned.
    pub(crate) fn root(&self) -> PathBuf {
        self.root.clone()
    }

    /// Reports whether the bounded directory traversal limit has been reached.
    pub(crate) fn directories_reached(&self, limit: usize) -> bool {
        self.directories >= limit
    }

    /// Accounts for one directory before its entries are inspected.
    pub(crate) fn count_directory(&mut self) {
        self.directories += 1;
    }

    /// Reports whether a hard result or scan limit requires traversal to stop.
    pub(crate) fn should_stop(&self) -> bool {
        self.stop_requested
    }

    /// Returns the remaining scan budget without allowing arithmetic overflow.
    pub(crate) fn remaining_bytes(&self, limit: usize) -> usize {
        limit.saturating_sub(self.scanned_bytes)
    }

    /// Accounts for bytes examined; callers pass only bytes actually read.
    pub(crate) fn consume_bytes(&mut self, count: usize) {
        self.scanned_bytes = self.scanned_bytes.saturating_add(count);
    }

    /// Records one bounded result per matching line and stops at the result cap.
    pub(crate) fn record_matches(&mut self, path: &str, text: &str) {
        for (index, line) in text.lines().enumerate() {
            if line.contains(&self.pattern) && !self.add_match(path, index + 1, line) {
                break;
            }
        }
    }

    /// Marks partial file input without necessarily stopping other files.
    pub(crate) fn mark_scan_limit(&mut self) {
        self.truncated = true;
    }

    /// Marks a limit that makes further scanning unable to produce a complete result.
    pub(crate) fn mark_hard_limit(&mut self) {
        self.truncated = true;
        self.stop_requested = true;
    }

    /// Adds a bounded warning while retaining successful no-match semantics.
    pub(crate) fn warning(&mut self, warning: String) {
        if self.warnings.len() < MAX_WARNINGS {
            let end = utf8_prefix(&warning, MAX_WARNING_BYTES);
            self.warnings.push(warning[..end].to_string());
        } else {
            self.warnings_truncated = true;
        }
    }

    /// Finalizes a success response; no-match remains an empty successful result.
    pub(crate) fn output(mut self, path: String) -> SearchFilesOutput {
        self.prepare_warnings();
        let warnings = (!self.warnings.is_empty()).then_some(self.warnings);
        SearchFilesOutput {
            action: Action::Searched,
            ok: true,
            pattern: self.pattern,
            path,
            match_count: self.matches.len(),
            matches: self.matches,
            truncated: self.truncated,
            next_offset: None,
            message: self.truncated.then(|| TRUNCATION_MESSAGE.to_string()),
            warnings,
        }
    }

    #[cfg(test)]
    pub(crate) fn match_count(&self) -> usize {
        self.matches.len()
    }

    #[cfg(test)]
    pub(crate) fn is_truncated(&self) -> bool {
        self.truncated
    }

    fn add_match(&mut self, path: &str, line: usize, content: &str) -> bool {
        let content = bounded_content(content);
        let size = path.len().saturating_add(content.len()).saturating_add(48);
        if self.matches.len() >= MAX_MATCHES_LIMIT
            || self.output_bytes.saturating_add(size) > MAX_MATCH_OUTPUT_BYTES
        {
            self.mark_hard_limit();
            return false;
        }
        self.output_bytes = self.output_bytes.saturating_add(size);
        self.matches.push(SearchMatch {
            path: path.to_string(),
            line,
            content,
        });
        true
    }

    fn prepare_warnings(&mut self) {
        if self.warnings_truncated {
            if self.warnings.len() == MAX_WARNINGS {
                self.warnings.pop();
            }
            self.warnings
                .push("Additional search warnings were omitted.".to_string());
        }
    }
}

fn bounded_content(content: &str) -> String {
    let end = utf8_prefix(content, MAX_MATCH_CONTENT_BYTES);
    content[..end].to_string()
}
