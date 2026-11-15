use crate::entities::{
    agent_event::payload::{PermissionRequestView, QuestionView},
    tool_confirm_result::outcome::ToolConfirmResult,
};

/// Safe default selected by Escape in the exit confirmation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ExitChoice {
    /// Keep the process running.
    #[default]
    Cancel,
    /// Stop the agent and leave after the safe boundary.
    Exit,
}

impl ExitChoice {
    /// Returns the next choice for keyboard navigation.
    pub(crate) const fn next(self) -> Self {
        match self {
            Self::Cancel => Self::Exit,
            Self::Exit => Self::Cancel,
        }
    }
}

/// One overlay that captures keyboard input until it is resolved.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ModalView {
    /// Confirmation shown for `Ctrl+C` and `Ctrl+Q`.
    ConfirmExit {
        /// Currently highlighted choice.
        selected: ExitChoice,
    },
    /// Confirmation shown before deleting the current history.
    ConfirmNew {
        /// Currently highlighted choice.
        selected: ExitChoice,
    },
    /// Multi-line goal text entered before starting an autonomous run.
    Goal {
        /// Exact text entered so far.
        input: String,
        /// Whether the last confirmation was rejected as empty.
        error: bool,
    },
    /// Tool permission request with its complete arguments.
    Permission {
        /// Agent-supplied request payload.
        request: PermissionRequestView,
        /// Index into the fixed permission choices.
        selected: usize,
        /// Vertical scroll offset for the argument preview.
        scroll: u16,
    },
    /// Model-authored question. An empty answer list asks for free-form text.
    Question {
        /// Agent-supplied question payload.
        request: QuestionView,
        /// Index into the answer list, or the free-form pseudo-item.
        selected: usize,
        /// Text typed for the free-form pseudo-item.
        input: String,
    },
}

impl ModalView {
    /// Creates the exit confirmation with the safe default highlighted.
    pub(crate) const fn confirm_exit() -> Self {
        Self::ConfirmExit {
            selected: ExitChoice::Cancel,
        }
    }

    /// Creates the new-conversation confirmation with cancel highlighted.
    pub(crate) const fn confirm_new() -> Self {
        Self::ConfirmNew {
            selected: ExitChoice::Cancel,
        }
    }

    /// Creates an empty goal input modal.
    pub(crate) fn goal() -> Self {
        Self::Goal {
            input: String::new(),
            error: false,
        }
    }

    /// Creates a permission modal with `Deny` highlighted.
    pub(crate) fn permission(request: PermissionRequestView) -> Self {
        Self::Permission {
            request,
            selected: 0,
            scroll: 0,
        }
    }

    /// Creates a question modal with the first answer highlighted.
    pub(crate) fn question(request: QuestionView) -> Self {
        Self::Question {
            request,
            selected: 0,
            input: String::new(),
        }
    }

    /// Returns the permission decision for the current selection.
    pub(crate) fn permission_decision(&self) -> Option<ToolConfirmResult> {
        match self {
            Self::Permission { selected, .. } => Some(ToolConfirmResult::from_index(*selected)),
            Self::ConfirmExit { .. }
            | Self::ConfirmNew { .. }
            | Self::Question { .. }
            | Self::Goal { .. } => None,
        }
    }

    /// Returns the correlation id owned by the modal, if it has one.
    pub(crate) const fn correlation_id(&self) -> Option<u64> {
        match self {
            Self::Permission { request, .. } => Some(request.correlation_id),
            Self::Question { request, .. } => Some(request.correlation_id),
            Self::ConfirmExit { .. } | Self::ConfirmNew { .. } | Self::Goal { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ExitChoice, ModalView};
    use crate::entities::{
        agent_event::payload::{InteractionOrigin, PermissionRequestView, QuestionView},
        tool_confirm_result::outcome::ToolConfirmResult,
    };

    #[test]
    fn confirm_exit_starts_on_the_safe_choice() {
        assert_eq!(
            ModalView::confirm_exit(),
            ModalView::ConfirmExit {
                selected: ExitChoice::Cancel
            }
        );
    }

    #[test]
    fn permission_modal_selects_every_decision_by_index() {
        let modal = ModalView::permission(PermissionRequestView::new(
            7,
            "read_file".to_string(),
            serde_json::json!({"path": "src/main.rs"}),
            InteractionOrigin::MainAgent,
        ));

        assert_eq!(modal.permission_decision(), Some(ToolConfirmResult::Deny));
        assert_eq!(modal.correlation_id(), Some(7));
    }

    #[test]
    fn free_form_question_has_no_answers() {
        let modal = ModalView::question(QuestionView::new(
            9,
            "Type an answer".to_string(),
            Vec::new(),
            InteractionOrigin::Subagent,
        ));

        assert_eq!(modal.correlation_id(), Some(9));
    }
}
