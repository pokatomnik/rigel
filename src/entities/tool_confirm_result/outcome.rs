use std::fmt::Display;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum ToolConfirmResult {
    /// Do not execute the tool call.
    #[default]
    Deny,

    /// Execute this tool call once.
    AllowOnce,

    /// Allow this tool for the current project.
    AllowForProject,

    /// Allow this tool for the current agent session.
    AllowForSession,

    /// Allow this tool globally.
    AllowForever,
}

impl ToolConfirmResult {
    /// Every decision in the order shown by the permission modal.
    pub(crate) const ALL: [Self; 5] = [
        Self::Deny,
        Self::AllowOnce,
        Self::AllowForProject,
        Self::AllowForSession,
        Self::AllowForever,
    ];

    /// Resolves a zero-based modal selection, clamping unknown indices to `Deny`.
    pub(crate) fn from_index(index: usize) -> Self {
        Self::ALL.get(index).copied().unwrap_or_default()
    }
}

impl Display for ToolConfirmResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolConfirmResult::Deny => f.write_str("Deny"),
            ToolConfirmResult::AllowOnce => f.write_str("Allow once"),
            ToolConfirmResult::AllowForProject => f.write_str("Allow for project"),
            ToolConfirmResult::AllowForSession => f.write_str("Allow for session"),
            ToolConfirmResult::AllowForever => f.write_str("Allow forever"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ToolConfirmResult;

    #[test]
    fn default_result_is_deny() {
        assert_eq!(ToolConfirmResult::default(), ToolConfirmResult::Deny);
    }

    #[test]
    fn modal_indices_round_trip_through_every_decision() {
        for (index, decision) in ToolConfirmResult::ALL.iter().enumerate() {
            assert_eq!(ToolConfirmResult::from_index(index), *decision);
        }
        assert_eq!(ToolConfirmResult::from_index(99), ToolConfirmResult::Deny);
    }

    #[test]
    fn result_labels_are_stable() {
        let results = [
            ToolConfirmResult::Deny,
            ToolConfirmResult::AllowOnce,
            ToolConfirmResult::AllowForProject,
            ToolConfirmResult::AllowForSession,
            ToolConfirmResult::AllowForever,
        ];
        let labels = results.iter().map(ToString::to_string).collect::<Vec<_>>();

        assert_eq!(
            labels,
            [
                "Deny",
                "Allow once",
                "Allow for project",
                "Allow for session",
                "Allow forever",
            ]
        );
    }
}
