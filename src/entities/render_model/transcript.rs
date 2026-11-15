use crate::entities::{application::input_reaction::ScrollIntent, transcript::entry::EntryId};

/// Viewport state owned by presentation so new rows never move a reader's anchor.
pub(crate) struct TranscriptScroll {
    offset: usize,
    follow_tail: bool,
    pending: Option<ScrollIntent>,
}

impl TranscriptScroll {
    /// Creates a viewport that follows new rows.
    pub(crate) fn new() -> Self {
        Self {
            offset: 0,
            follow_tail: true,
            pending: None,
        }
    }

    /// Queues one viewport movement applied on the next frame.
    pub(crate) fn request(&mut self, intent: ScrollIntent) {
        self.pending = Some(intent);
    }

    /// Resolves the queued movement against the current wrapped row count.
    pub(crate) fn resolve(&mut self, total: usize, height: usize) -> u16 {
        let max = total.saturating_sub(height.max(1));
        match self.pending.take() {
            Some(ScrollIntent::PageUp) => self.page_up(max, height),
            Some(ScrollIntent::PageDown) => self.page_down(max, height),
            Some(ScrollIntent::Tail) => self.follow_tail = true,
            None => {}
        }
        if self.follow_tail {
            self.offset = max;
        }
        self.offset.min(max).min(u16::MAX as usize) as u16
    }

    fn page_up(&mut self, max: usize, height: usize) {
        if self.follow_tail {
            self.offset = max;
            self.follow_tail = false;
        }
        self.offset = self.offset.saturating_sub(height.max(2) - 1);
    }

    fn page_down(&mut self, max: usize, height: usize) {
        let page = height.max(2) - 1;
        self.offset = self.offset.saturating_add(page).min(max);
        self.follow_tail = self.offset == max;
    }
}

/// Transcript rows and presentation-only expansion state.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct TranscriptView {
    /// Rows in display order.
    pub(crate) rows: Vec<TranscriptRow>,
    /// Entries whose collapsible content is currently visible.
    pub(crate) expanded: Vec<EntryId>,
}

impl TranscriptView {
    /// Creates a transcript view with explicit expansion state.
    pub(crate) fn new(rows: Vec<TranscriptRow>, expanded: Vec<EntryId>) -> Self {
        Self { rows, expanded }
    }

    /// Reports whether a collapsible entry is expanded.
    pub(crate) fn is_expanded(&self, id: EntryId) -> bool {
        self.expanded.contains(&id)
    }
}

/// One transcript row prepared for rendering.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum TranscriptRow {
    /// User prompt with its stable entry identity.
    User {
        /// Entry identity used by expansion state.
        id: EntryId,
        /// Exact prompt text.
        text: String,
    },
    /// Assistant answer with optional reasoning.
    Assistant {
        /// Entry identity used by expansion state.
        id: EntryId,
        /// Final answer text.
        text: String,
        /// Reasoning content shown only when expanded.
        reasoning: Option<String>,
    },
    /// Streamed answer text that has not become a durable entry yet.
    LiveAssistant {
        /// Text received so far in the current turn.
        text: String,
    },
    /// Streamed reasoning that has not become a durable entry yet.
    LiveReasoning {
        /// Reasoning text received so far in the current turn.
        text: String,
        /// Whether the user expanded the live block manually.
        expanded: bool,
    },
    /// Tool call block.
    Tool(ToolRow),
    /// Compaction summary block.
    Summary {
        /// Entry identity used by expansion state.
        id: EntryId,
        /// Exact summary text.
        text: String,
    },
    /// Boundary that points to a sealed archive segment.
    Boundary {
        /// Entry identity used by expansion state.
        id: EntryId,
        /// Archive file name.
        archive: String,
    },
}

/// Tool call block with its full arguments and optional result.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ToolRow {
    /// Entry identity used by expansion state.
    pub(crate) id: EntryId,
    /// Exact runtime tool name.
    pub(crate) tool_name: String,
    /// Complete pretty-printed JSON arguments.
    pub(crate) arguments: String,
    /// Result shown only after the tool finished.
    pub(crate) result: Option<ToolResultRow>,
    /// Isolated child-session details for `spawn_subagent`.
    pub(crate) subagent: Option<SubagentRow>,
}

impl ToolRow {
    /// Creates a collapsed tool block without a result.
    pub(crate) fn new(id: EntryId, tool_name: String, arguments: String) -> Self {
        Self {
            id,
            tool_name,
            arguments,
            result: None,
            subagent: None,
        }
    }

    /// Attaches the completed tool result.
    pub(crate) fn with_result(mut self, result: ToolResultRow) -> Self {
        self.result = Some(result);
        self
    }

    /// Attaches isolated subagent work shown when the block is expanded.
    pub(crate) fn with_subagent(mut self, subagent: SubagentRow) -> Self {
        self.subagent = Some(subagent);
        self
    }
}

/// Isolated delegated work rendered inside one `spawn_subagent` block.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub(crate) struct SubagentRow {
    /// Complete delegated task text.
    pub(crate) prompt: String,
    /// Child answer, reasoning, and tool activity accumulated during the run.
    pub(crate) activity: String,
    /// Final report or actionable failure text once the child finished.
    pub(crate) report: Option<String>,
}

impl SubagentRow {
    /// Creates an empty subagent row for the delegated task.
    pub(crate) fn new(prompt: String) -> Self {
        Self {
            prompt,
            activity: String::new(),
            report: None,
        }
    }
}

/// Completed tool result rendered inside an expanded tool block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ToolResultRow {
    /// Complete result payload without truncation.
    pub(crate) output: String,
    /// Whether the tool reported an actionable failure.
    pub(crate) failed: bool,
}

impl ToolResultRow {
    /// Creates a completed result with its failure flag.
    pub(crate) fn new(output: String, failed: bool) -> Self {
        Self { output, failed }
    }
}

#[cfg(test)]
mod tests {
    use super::{ToolResultRow, ToolRow, TranscriptRow, TranscriptScroll, TranscriptView};
    use crate::entities::{application::input_reaction::ScrollIntent, transcript::entry::EntryId};

    #[test]
    fn expansion_state_is_looked_up_by_stable_id() {
        let view = TranscriptView::new(Vec::new(), vec![EntryId::new(3)]);

        assert!(view.is_expanded(EntryId::new(3)));
        assert!(!view.is_expanded(EntryId::new(4)));
    }

    #[test]
    fn tool_rows_start_collapsed_and_accept_one_result() {
        let row = ToolRow::new(EntryId::new(2), "read_file".to_string(), "{}".to_string());
        assert!(row.result.is_none());
        let row = row.with_result(ToolResultRow::new("contents".to_string(), false));

        assert_eq!(
            row.result,
            Some(ToolResultRow::new("contents".to_string(), false))
        );
    }

    #[test]
    fn page_up_leaves_the_tail_and_page_down_returns_to_it() {
        let mut scroll = TranscriptScroll::new();
        assert_eq!(scroll.resolve(100, 10), 90);

        scroll.request(ScrollIntent::PageUp);
        assert_eq!(scroll.resolve(100, 10), 81);

        scroll.request(ScrollIntent::PageDown);
        assert_eq!(scroll.resolve(100, 10), 90);
    }

    #[test]
    fn a_scrolled_anchor_does_not_move_when_rows_are_appended() {
        let mut scroll = TranscriptScroll::new();
        scroll.request(ScrollIntent::PageUp);
        assert_eq!(scroll.resolve(100, 10), 81);

        assert_eq!(scroll.resolve(120, 10), 81);
    }

    #[test]
    fn user_rows_keep_their_identity() {
        let row = TranscriptRow::User {
            id: EntryId::new(7),
            text: "hello".to_string(),
        };

        assert_eq!(
            row,
            TranscriptRow::User {
                id: EntryId::new(7),
                text: "hello".to_string(),
            }
        );
    }
}
