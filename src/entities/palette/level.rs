use serde::{Deserialize, Serialize};

/// One screen of the two-level command palette.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum PaletteLevel {
    /// Root commands and context actions.
    #[default]
    Root,
    /// Model selection list.
    Models,
    /// MCP server status list.
    McpServers,
    /// Skill selection list.
    Skills,
}

impl PaletteLevel {
    /// Returns the parent level for nested screens, or `None` at the root.
    pub(crate) const fn parent(self) -> Option<Self> {
        match self {
            Self::Root => None,
            Self::Models | Self::McpServers | Self::Skills => Some(Self::Root),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PaletteLevel;

    #[test]
    fn only_nested_levels_have_a_parent() {
        assert_eq!(PaletteLevel::Root.parent(), None);
        assert_eq!(PaletteLevel::Models.parent(), Some(PaletteLevel::Root));
        assert_eq!(PaletteLevel::McpServers.parent(), Some(PaletteLevel::Root));
        assert_eq!(PaletteLevel::Skills.parent(), Some(PaletteLevel::Root));
    }

    #[test]
    fn root_is_the_default_level() {
        assert_eq!(PaletteLevel::default(), PaletteLevel::Root);
    }
}
