use std::path::{Path, PathBuf};

/// Selects the configuration profile used to start one chat session.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ChatRequest {
    profile: Option<PathBuf>,
}

impl ChatRequest {
    /// Creates a request using the default configuration profile when `profile` is absent.
    pub(crate) fn new(profile: Option<PathBuf>) -> Self {
        Self { profile }
    }

    /// Returns the explicitly requested profile path, if one was supplied.
    pub(crate) fn profile_path(&self) -> Option<&Path> {
        self.profile.as_deref()
    }
}
