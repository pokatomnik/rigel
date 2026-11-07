/// Identifies why a new isolated model session is being requested.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ModelSessionKind {
    /// Initial interactive chat session.
    Chat,
    /// Replacement after `/agent`.
    ModelChange,
    /// Fresh non-persistent delegated task.
    Subagent,
}
