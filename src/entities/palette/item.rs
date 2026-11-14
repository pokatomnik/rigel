use serde::{Deserialize, Serialize};

use crate::entities::agent_command::command::AgentCommand;

use super::level::PaletteLevel;

/// Payload that can be copied from the transcript through a palette action.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum CopyTarget {
    /// Last assistant answer.
    Answer,
    /// Last collapsible block in its full form.
    Block,
    /// Last compaction summary.
    Summary,
}

/// What happens when a palette item is activated.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum PaletteAction {
    /// Opens a nested palette level.
    Open(PaletteLevel),
    /// Returns to the parent palette level.
    Back,
    /// Sends an agent command.
    Command(AgentCommand),
    /// Copies a transcript payload to the clipboard.
    Copy(CopyTarget),
    /// Requests a retry of a failed durable history write.
    RetryPersistence,
    /// Scrolls the transcript to the newest entry.
    JumpToLatest,
    /// Opens the exit confirmation instead of sending the command immediately.
    ConfirmExit,
    /// Opens the new-conversation confirmation.
    ConfirmNewConversation,
    /// Opens the goal input modal.
    OpenGoal,
    /// Dismisses every transient notice.
    DismissNotices,
    /// Informational row that performs no action when activated.
    Info,
}

/// Semantic color assigned to one palette row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ItemTone {
    /// Ordinary text color.
    #[default]
    Default,
    /// Healthy or current state.
    Success,
    /// Neutral pending state.
    Muted,
    /// Failed state.
    Error,
}

/// One selectable row in the command palette.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PaletteItem {
    /// Stable identifier used for selection and future shortcuts.
    pub(crate) id: String,
    /// Primary label shown in the list.
    pub(crate) label: String,
    /// Optional secondary text shown in the muted color.
    pub(crate) detail: Option<String>,
    /// Whether the item can currently be activated.
    pub(crate) enabled: bool,
    /// Semantic tone used by the renderer.
    pub(crate) tone: ItemTone,
    /// Whether the detail is aligned to the right edge of the list.
    pub(crate) detail_right: bool,
    /// Action executed when the user presses Enter.
    pub(crate) action: PaletteAction,
}

impl PaletteItem {
    /// Creates an enabled palette item with no secondary text.
    pub(crate) fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        action: PaletteAction,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            detail: None,
            enabled: true,
            tone: ItemTone::default(),
            detail_right: false,
            action,
        }
    }

    /// Adds secondary text without changing the action.
    pub(crate) fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Assigns the semantic color used for the row text and detail.
    pub(crate) fn with_tone(mut self, tone: ItemTone) -> Self {
        self.tone = tone;
        self
    }

    /// Aligns the detail to the right edge, used for model context sizes.
    pub(crate) fn with_right_detail(mut self) -> Self {
        self.detail_right = true;
        self
    }

    /// Marks the item as visible but not activatable.
    pub(crate) fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::{CopyTarget, PaletteAction, PaletteItem};
    use crate::entities::{agent_command::command::AgentCommand, palette::level::PaletteLevel};

    #[test]
    fn builders_preserve_label_detail_and_enabled_state() {
        let item = PaletteItem::new(
            "copy-answer",
            "Copy last answer",
            PaletteAction::Copy(CopyTarget::Answer),
        )
        .with_detail("Ctrl+Shift+C later");

        assert_eq!(item.label, "Copy last answer");
        assert_eq!(item.detail.as_deref(), Some("Ctrl+Shift+C later"));
        assert!(item.enabled);
        assert!(!item.disabled().enabled);
    }

    #[test]
    fn nested_navigation_items_keep_their_levels() {
        let open = PaletteItem::new(
            "models",
            "Change model",
            PaletteAction::Open(PaletteLevel::Models),
        );
        let back = PaletteItem::new("back", "← Back", PaletteAction::Back);

        assert_eq!(open.action, PaletteAction::Open(PaletteLevel::Models));
        assert_eq!(back.action, PaletteAction::Back);
    }

    #[test]
    fn command_actions_round_trip_through_json() -> anyhow::Result<()> {
        let item = PaletteItem::new(
            "compact",
            "Compact context",
            PaletteAction::Command(AgentCommand::Compact),
        );
        let json = serde_json::to_value(&item)?;
        let restored: PaletteItem = serde_json::from_value(json)?;

        assert_eq!(restored, item);
        Ok(())
    }
}
