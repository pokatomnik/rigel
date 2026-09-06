use std::path::Path;

use rig::tool::ToolExecutionError;

use crate::tools::utils::{
    errors::{invalid_argument, path_outside_workspace},
    path::MAX_PATH_BYTES,
};

/// Parsed relative glob pattern with the server's intentionally small dialect.
pub(super) struct GlobPattern {
    normalized: String,
    segments: Vec<String>,
}

impl GlobPattern {
    /// Parses and normalizes a relative pattern before filesystem access.
    pub(super) fn parse(value: &str) -> Result<Self, ToolExecutionError> {
        Self::validate_input(value)?;
        let value = value.replace('\\', "/");
        if Self::is_absolute(&value) {
            return Err(path_outside_workspace(&value, "glob"));
        }
        let segments = Self::parse_segments(&value)?;
        let normalized = segments.join("/");
        Ok(Self {
            normalized,
            segments,
        })
    }

    fn validate_input(value: &str) -> Result<(), ToolExecutionError> {
        if value.is_empty() || value.contains('\0') || value.len() > MAX_PATH_BYTES {
            return Err(invalid_argument(
                "Cannot glob files: pattern must be non-empty, contain no NUL characters, and be at most 4096 bytes.",
            ));
        }
        Ok(())
    }

    fn parse_segments(value: &str) -> Result<Vec<String>, ToolExecutionError> {
        let mut segments = Vec::new();
        for segment in value.split('/') {
            if segment.is_empty() || segment == "." {
                continue;
            }
            if segment == ".." {
                return Err(path_outside_workspace(value, "glob"));
            }
            if segment.contains(['[', ']', '{', '}']) {
                return Err(invalid_argument(
                    "Cannot glob files: only *, **, ?, and path separators are supported.",
                ));
            }
            segments.push(segment.to_string());
        }
        if segments.is_empty() {
            return Err(invalid_argument(
                "Cannot glob files: pattern must identify a relative file path.",
            ));
        }
        Ok(segments)
    }

    /// Returns the normalized pattern used in the model-facing result.
    pub(super) fn normalized(&self) -> &str {
        &self.normalized
    }

    /// Matches one normalized workspace-relative path without matching separators in `*`.
    pub(super) fn matches(&self, path: &str) -> bool {
        let path_segments = path.split('/').collect::<Vec<_>>();
        let mut suffix = vec![false; path_segments.len() + 1];
        suffix[path_segments.len()] = true;
        for pattern in self.segments.iter().rev() {
            let mut current = vec![false; path_segments.len() + 1];
            if pattern == "**" {
                current[path_segments.len()] = suffix[path_segments.len()];
                for index in (0..path_segments.len()).rev() {
                    current[index] = suffix[index] || current[index + 1];
                }
            } else {
                for index in 0..path_segments.len() {
                    current[index] =
                        suffix[index + 1] && Self::matches_segment(pattern, path_segments[index]);
                }
            }
            suffix = current;
        }
        suffix[0]
    }

    fn is_absolute(value: &str) -> bool {
        let bytes = value.as_bytes();
        Path::new(value).is_absolute()
            || value.starts_with("//")
            || (bytes.len() >= 2 && bytes[1] == b':')
    }

    fn matches_segment(pattern: &str, value: &str) -> bool {
        let pattern = pattern.chars().collect::<Vec<_>>();
        let value = value.chars().collect::<Vec<_>>();
        let mut pattern_index = 0;
        let mut value_index = 0;
        let mut star_index = None;
        let mut star_value = 0;
        while value_index < value.len() {
            if pattern_index < pattern.len()
                && (pattern[pattern_index] == '?' || pattern[pattern_index] == value[value_index])
            {
                pattern_index += 1;
                value_index += 1;
            } else if pattern_index < pattern.len() && pattern[pattern_index] == '*' {
                star_index = Some(pattern_index);
                pattern_index += 1;
                star_value = value_index;
            } else if let Some(star) = star_index {
                star_value += 1;
                value_index = star_value;
                pattern_index = star + 1;
            } else {
                return false;
            }
        }
        while pattern_index < pattern.len() && pattern[pattern_index] == '*' {
            pattern_index += 1;
        }
        pattern_index == pattern.len()
    }
}
