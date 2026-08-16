use std::fmt::Display;

#[derive(Clone, Copy, Default)]
pub(crate) enum ToolConfirmResult {
    /// Do not execute the tool call.
    #[default]
    No,

    /// Execute this tool call once.
    AllowOnce,
}

impl Display for ToolConfirmResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolConfirmResult::No => f.write_str("No"),
            ToolConfirmResult::AllowOnce => f.write_str("Allow once"),
        }
    }
}
