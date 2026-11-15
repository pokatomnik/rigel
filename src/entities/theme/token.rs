use serde::{Deserialize, Serialize};

/// Semantic color roles shared by every rendered element.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum ColorToken {
    /// Main application background.
    Background,
    /// Raised surfaces such as the composer, popups, and expanded details.
    Surface,
    /// Highlighted list row or selected region.
    Selection,
    /// Primary readable text.
    Text,
    /// Secondary text, metadata, and collapsed reasoning.
    Muted,
    /// Focus, actions, and the user marker.
    Accent,
    /// Completed states and healthy MCP servers.
    Success,
    /// Attention states and skipped data.
    Warning,
    /// Errors and failed MCP servers.
    Error,
    /// Structural separators and borders; never body text.
    Border,
}

impl ColorToken {
    /// Every token in the order documented by the accepted palette.
    #[cfg(test)]
    pub(crate) const ALL: [Self; 10] = [
        Self::Background,
        Self::Surface,
        Self::Selection,
        Self::Text,
        Self::Muted,
        Self::Accent,
        Self::Success,
        Self::Warning,
        Self::Error,
        Self::Border,
    ];

    /// Returns the stable index of the token inside palette storage.
    #[cfg(test)]
    pub(crate) const fn index(self) -> usize {
        match self {
            Self::Background => 0,
            Self::Surface => 1,
            Self::Selection => 2,
            Self::Text => 3,
            Self::Muted => 4,
            Self::Accent => 5,
            Self::Success => 6,
            Self::Warning => 7,
            Self::Error => 8,
            Self::Border => 9,
        }
    }

    /// Returns the documented token name used by diagnostics and future theme files.
    #[cfg(test)]
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Background => "background",
            Self::Surface => "surface",
            Self::Selection => "selection",
            Self::Text => "text",
            Self::Muted => "muted",
            Self::Accent => "accent",
            Self::Success => "success",
            Self::Warning => "warning",
            Self::Error => "error",
            Self::Border => "border",
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::ColorToken;

    #[test]
    fn every_token_has_a_unique_index() {
        let indices = ColorToken::ALL
            .iter()
            .map(|token| token.index())
            .collect::<HashSet<_>>();

        assert_eq!(indices.len(), ColorToken::ALL.len());
        assert_eq!(indices, (0..ColorToken::ALL.len()).collect());
    }

    #[test]
    fn every_token_has_a_unique_name() {
        let names = ColorToken::ALL
            .iter()
            .map(|token| token.name())
            .collect::<HashSet<_>>();

        assert_eq!(names.len(), ColorToken::ALL.len());
    }
}
