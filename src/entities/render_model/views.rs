use crate::entities::{
    agent_event::status::{AgentActivity, Notice, ReadOnlyState},
    conversation::context_usage::ContextUsage,
};

/// Header information rendered in the first row.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HeaderView {
    /// Selected model identifier, if a model is configured.
    pub(crate) model: Option<String>,
    /// MCP connection aggregate.
    pub(crate) mcp: McpSummary,
    /// Latest provider-neutral context usage.
    pub(crate) context: ContextUsage,
    /// High-level agent activity.
    pub(crate) activity: AgentActivity,
}

impl HeaderView {
    /// Creates a header with no MCP servers and an idle agent.
    pub(crate) fn new(model: Option<String>, context: ContextUsage) -> Self {
        Self {
            model,
            mcp: McpSummary::default(),
            context,
            activity: AgentActivity::Idle,
        }
    }

    /// Adds the MCP aggregate.
    pub(crate) fn with_mcp(mut self, mcp: McpSummary) -> Self {
        self.mcp = mcp;
        self
    }

    /// Adds the current activity state.
    pub(crate) fn with_activity(mut self, activity: AgentActivity) -> Self {
        self.activity = activity;
        self
    }
}

/// Aggregate of connected and configured MCP servers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct McpSummary {
    /// Number of servers that are running.
    pub(crate) running: usize,
    /// Number of servers still connecting.
    pub(crate) starting: usize,
    /// Number of configured servers.
    pub(crate) total: usize,
}

impl McpSummary {
    /// Creates an aggregate from running and total counts.
    pub(crate) fn new(running: usize, total: usize) -> Self {
        Self {
            running,
            total,
            starting: 0,
        }
    }

    /// Adds the number of servers that are still connecting.
    pub(crate) fn with_starting(mut self, starting: usize) -> Self {
        self.starting = starting;
        self
    }
}

/// Formats a token count as the compact label used by the header and model list.
pub(crate) fn format_token_count(value: u64) -> String {
    if value < 1_000 {
        return value.to_string();
    }
    if value < 1_000_000 {
        return format!("{}K", value / 1_000);
    }
    if value < 1_000_000_000 {
        return format!("{}M", value / 1_000_000);
    }
    if value < 1_000_000_000_000 {
        return format!("{}B", value / 1_000_000_000);
    }
    value.to_string()
}

/// Composer text and focus state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ComposerView {
    /// Current draft text.
    pub(crate) text: String,
    /// Whether the text editor owns focus.
    pub(crate) focused: bool,
    /// Whether the submit control owns focus.
    pub(crate) submit_focused: bool,
}

impl ComposerView {
    /// Creates a composer containing the provided draft.
    pub(crate) fn new(text: String) -> Self {
        Self {
            text,
            focused: true,
            submit_focused: false,
        }
    }
}

/// Status line and transient notices.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StatusView {
    /// High-level agent activity.
    pub(crate) activity: AgentActivity,
    /// Durable-write availability.
    pub(crate) read_only: ReadOnlyState,
    /// Notices shown next to the status.
    pub(crate) notices: Vec<Notice>,
    /// Optional contextual hint.
    pub(crate) hint: Option<String>,
}

impl StatusView {
    /// Creates a status line with activity and write availability.
    pub(crate) fn new(activity: AgentActivity, read_only: ReadOnlyState) -> Self {
        Self {
            activity,
            read_only,
            notices: Vec::new(),
            hint: None,
        }
    }

    /// Adds transient notices.
    pub(crate) fn with_notices(mut self, notices: Vec<Notice>) -> Self {
        self.notices = notices;
        self
    }

    /// Adds a contextual hint.
    pub(crate) fn with_hint(mut self, hint: Option<String>) -> Self {
        self.hint = hint;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::{ComposerView, HeaderView, McpSummary, StatusView};
    use crate::entities::{
        agent_event::status::{AgentActivity, ReadOnlyState},
        conversation::context_usage::ContextUsage,
    };

    #[test]
    fn header_builders_keep_model_and_usage() {
        let header = HeaderView::new(
            Some("model-a".to_string()),
            ContextUsage::new(Some(1), Some(10)),
        )
        .with_mcp(McpSummary::new(1, 2))
        .with_activity(AgentActivity::Running);

        assert_eq!(header.model.as_deref(), Some("model-a"));
        assert_eq!(header.mcp, McpSummary::new(1, 2));
        assert_eq!(header.activity, AgentActivity::Running);
    }

    #[test]
    fn composer_starts_focused_on_the_editor() {
        let composer = ComposerView::new("hello".to_string());

        assert_eq!(composer.text, "hello");
        assert!(composer.focused);
        assert!(!composer.submit_focused);
    }

    #[test]
    fn status_builders_add_notices_and_hints() {
        let status = StatusView::new(AgentActivity::Stopping, ReadOnlyState::writable())
            .with_hint(Some("hint".to_string()));

        assert_eq!(status.activity, AgentActivity::Stopping);
        assert_eq!(status.hint.as_deref(), Some("hint"));
    }
}
