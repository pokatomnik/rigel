use std::fmt::Display;

#[derive(Clone, Copy)]
pub(crate) enum ToolConfirmResult {
    /// Allow once
    YesOnce,

    /// Allow and remember for a current directory
    // TODO implement later
    // YesProject,

    /// Allow and never ask again for the OS user
    // TODO implement later
    // YesGlobal,

    /// Forbid
    No,
}

impl Default for ToolConfirmResult {
    fn default() -> Self {
        return ToolConfirmResult::No;
    }
}

impl Display for ToolConfirmResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolConfirmResult::YesOnce => f.write_str("Yes (once)"),
            ToolConfirmResult::No => f.write_str("No"),
        }
    }
}
