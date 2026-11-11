use std::path::PathBuf;

use crate::entities::selected_model::selection::SelectedModel;

/// Defines whether a tool may run automatically or needs user confirmation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PermissionRequirement {
    /// The tool can run without a confirmation prompt.
    Automatic,
    /// The tool requires a policy decision or confirmation prompt.
    ConfirmationRequired,
}

/// Domain decision returned by the permission policy workflow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ToolPermissionDecision {
    /// Permit the tool call.
    Allow,
    /// Reject the tool call with an actionable model-facing reason.
    Deny(String),
}

/// Identifies one tool call when evaluating permission policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ToolPermissionRequest {
    tool_name: String,
}

impl ToolPermissionRequest {
    /// Creates a request from the runtime tool name, including unknown names safely.
    pub(crate) fn new(tool_name: String) -> Self {
        Self { tool_name }
    }

    /// Returns the exact runtime tool name used for policy lookup and diagnostics.
    pub(crate) fn tool_name(&self) -> &str {
        self.tool_name.as_str()
    }
}

/// Describes a bounded command execution requested by a model tool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CommandExecutionRequest {
    /// Shell command text to execute.
    pub(crate) command: String,
    /// Working directory selected by the composition root.
    pub(crate) current_dir: PathBuf,
}

/// Describes bounded command output without exposing process types across infrastructure traits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CommandExecutionResult {
    /// The real process exit code, or `-1` when no exit code was available.
    pub(crate) exit_code: i32,
    /// Combined bounded stdout and stderr.
    pub(crate) output: String,
    /// Whether execution exceeded its configured time budget.
    pub(crate) timed_out: bool,
    /// Whether the adapter discarded bytes beyond its output bound.
    pub(crate) truncated: bool,
}

/// A bounded HTTP request supplied to the safe network adapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HttpFetchRequest {
    /// Initial URL supplied by the model.
    pub(crate) url: String,
}

/// A bounded HTTP response returned by the safe network adapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HttpFetchResponse {
    /// HTTP status code.
    pub(crate) status: u16,
    /// Bounded UTF-8 body.
    pub(crate) body: String,
}

/// A task submitted to an isolated subagent session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SubagentTask {
    /// The model-authored task text.
    pub(crate) task: String,
    selected_model: Option<SelectedModel>,
}

impl SubagentTask {
    /// Creates a delegated task without model inheritance metadata.
    pub(crate) fn new(task: String) -> Self {
        Self {
            task,
            selected_model: None,
        }
    }

    /// Attaches the parent session's selected model for isolated child-session creation.
    pub(crate) fn with_selected_model(mut self, selected_model: SelectedModel) -> Self {
        self.selected_model = Some(selected_model);
        self
    }

    /// Returns the inherited model, if the parent turn supplied one.
    pub(crate) fn selected_model(&self) -> Option<&SelectedModel> {
        self.selected_model.as_ref()
    }
}

/// A successful report returned by a subagent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SubagentReport {
    /// The exact report text returned by the subagent.
    pub(crate) report: String,
}
