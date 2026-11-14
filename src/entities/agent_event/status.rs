use serde::{Deserialize, Serialize};

/// High-level activity state of the agent runtime.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum AgentActivity {
    /// No turn, compaction, or stop transition is in progress.
    #[default]
    Idle,
    /// A model turn or its tools are running.
    Running,
    /// A soft stop was requested and is waiting for a safe boundary.
    Stopping,
    /// Context compaction owns the session.
    Compacting,
}

impl AgentActivity {
    /// Reports whether commands that require a true idle state are currently allowed.
    pub(crate) const fn is_idle(self) -> bool {
        matches!(self, Self::Idle)
    }
}

/// Connection state of one configured MCP server.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum McpServerState {
    /// Connection is still being established.
    Starting,
    /// Tools from the server are available.
    Running,
    /// Connection failed and the server is excluded for this run.
    Failed,
}

/// One MCP server with the state shown in the header and palette.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct McpServerStatus {
    /// Configured server name.
    pub(crate) name: String,
    /// Observed connection state.
    pub(crate) state: McpServerState,
}

impl McpServerStatus {
    /// Creates a status snapshot for one configured server.
    pub(crate) fn new(name: String, state: McpServerState) -> Self {
        Self { name, state }
    }
}

/// Severity of a transient user-facing notice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum NoticeLevel {
    /// Informational message.
    Info,
    /// Attention message that does not stop the run.
    Warning,
    /// Failure message.
    Error,
    /// Successful completion message.
    Success,
}

/// A transient message that is not part of the durable conversation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Notice {
    /// Presentation severity.
    pub(crate) level: NoticeLevel,
    /// User-facing text.
    pub(crate) text: String,
}

impl Notice {
    /// Creates one transient notice.
    pub(crate) fn new(level: NoticeLevel, text: impl Into<String>) -> Self {
        Self {
            level,
            text: text.into(),
        }
    }
}

/// Lifecycle phase of the optional autonomous goal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum GoalPhase {
    /// No goal is active.
    Inactive,
    /// The agent is pursuing the goal.
    Active,
    /// The goal report is available.
    Completed,
}

/// Snapshot of the goal state shown by the UI.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct GoalView {
    /// Current lifecycle phase.
    pub(crate) phase: GoalPhase,
    /// Goal text or completion report, depending on the phase.
    pub(crate) text: Option<String>,
}

impl GoalView {
    /// Creates an inactive goal snapshot.
    pub(crate) fn inactive() -> Self {
        Self {
            phase: GoalPhase::Inactive,
            text: None,
        }
    }

    /// Creates an active goal snapshot with its text.
    pub(crate) fn active(text: impl Into<String>) -> Self {
        Self {
            phase: GoalPhase::Active,
            text: Some(text.into()),
        }
    }

    /// Creates a completed goal snapshot with its report.
    pub(crate) fn completed(report: impl Into<String>) -> Self {
        Self {
            phase: GoalPhase::Completed,
            text: Some(report.into()),
        }
    }
}

/// Durable-writing availability after a persistence failure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ReadOnlyState {
    /// Whether durable writes are blocked.
    pub(crate) active: bool,
    /// Actionable failure description shown next to the retry action.
    pub(crate) reason: Option<String>,
}

impl ReadOnlyState {
    /// Creates the writable state.
    pub(crate) fn writable() -> Self {
        Self {
            active: false,
            reason: None,
        }
    }

    /// Creates a blocked state with the persistence failure reason.
    pub(crate) fn read_only(reason: impl Into<String>) -> Self {
        Self {
            active: true,
            reason: Some(reason.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AgentActivity, GoalView, McpServerState, McpServerStatus, Notice, NoticeLevel,
        ReadOnlyState,
    };

    #[test]
    fn only_idle_activity_accepts_idle_only_commands() {
        assert!(AgentActivity::Idle.is_idle());
        assert!(!AgentActivity::Running.is_idle());
        assert!(!AgentActivity::Stopping.is_idle());
        assert!(!AgentActivity::Compacting.is_idle());
    }

    #[test]
    fn activity_serializes_with_a_stable_default() -> anyhow::Result<()> {
        assert_eq!(AgentActivity::default(), AgentActivity::Idle);
        let json = serde_json::to_value(AgentActivity::Stopping)?;

        assert_eq!(json, serde_json::json!("Stopping"));
        assert_eq!(
            serde_json::from_value::<AgentActivity>(json)?,
            AgentActivity::Stopping
        );
        Ok(())
    }

    #[test]
    fn mcp_status_preserves_name_and_state() {
        let status = McpServerStatus::new("github".to_string(), McpServerState::Failed);

        assert_eq!(status.name, "github");
        assert_eq!(status.state, McpServerState::Failed);
    }

    #[test]
    fn notices_keep_level_and_text() {
        let notice = Notice::new(NoticeLevel::Warning, "3 lines skipped");

        assert_eq!(notice.level, NoticeLevel::Warning);
        assert_eq!(notice.text, "3 lines skipped");
    }

    #[test]
    fn goal_views_cover_every_phase() {
        assert_eq!(GoalView::inactive().phase, super::GoalPhase::Inactive);
        assert_eq!(
            GoalView::active("finish the task").text.as_deref(),
            Some("finish the task")
        );
        assert_eq!(
            GoalView::completed("done").phase,
            super::GoalPhase::Completed
        );
    }

    #[test]
    fn read_only_state_distinguishes_blocked_writes() {
        assert!(!ReadOnlyState::writable().active);
        let blocked = ReadOnlyState::read_only("disk is full");

        assert!(blocked.active);
        assert_eq!(blocked.reason.as_deref(), Some("disk is full"));
    }
}
