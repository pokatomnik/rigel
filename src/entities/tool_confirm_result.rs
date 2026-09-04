use std::fmt::Display;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
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
