use std::fmt;

/// Identifies where a skill was discovered so duplicate names can be explained.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum SkillOrigin {
    /// A skill supplied by the current workspace.
    Project,
    /// A skill supplied by the user's home directory.
    Global,
    /// A skill bundled with Rigel.
    Builtin,
}

impl SkillOrigin {
    /// Returns the stable label used when duplicate skill names are displayed.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Global => "global",
            Self::Builtin => "builtin",
        }
    }
}

/// Metadata for a skill; storage and filesystem handles stay outside the entity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SkillDescriptor {
    name: String,
    origin: SkillOrigin,
    display_name: String,
}

impl SkillDescriptor {
    /// Creates a descriptor whose display name initially equals its domain name.
    pub(crate) fn new(name: String, origin: SkillOrigin) -> Self {
        Self {
            display_name: name.clone(),
            name,
            origin,
        }
    }

    /// Returns the stable skill identifier used by repository lookups.
    pub(crate) fn name(&self) -> &str {
        self.name.as_str()
    }

    /// Returns the source category of this skill.
    pub(crate) fn origin(&self) -> SkillOrigin {
        self.origin
    }

    /// Changes the display name when duplicate skill names need disambiguation.
    /// Changes only the presentation label when duplicate names need disambiguation.
    pub(crate) fn disambiguate(&mut self) {
        self.display_name = format!("{} ({})", self.name, self.origin.label());
    }
}

impl fmt::Display for SkillDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.display_name.as_str())
    }
}
