mod access;
mod copy;
mod subagent;
mod view;
use super::palette::PaletteState;
use crate::entities::{
    agent_event::{
        event::AgentEvent,
        payload::{PermissionRequestView, QuestionView},
        status::{AgentActivity, GoalView, McpServerStatus, Notice, NoticeLevel, ReadOnlyState},
    },
    application::input_event::TerminalSize,
    conversation::context_usage::ContextUsage,
    render_model::transcript::SubagentRow,
    selected_model::selection::SelectedModel,
    skill::definition::SkillDescriptor,
    transcript::entry::{EntryId, TranscriptEntry, TranscriptEntryKind},
};
use std::collections::HashMap;
/// Presentation state owned by the UI side of the event boundary.
pub(crate) struct UiState {
    size: TerminalSize,
    model: Option<String>,
    mcp: Vec<McpServerStatus>,
    models: Vec<SelectedModel>,
    skills: Vec<SkillDescriptor>,
    goal: GoalView,
    context: ContextUsage,
    activity: AgentActivity,
    read_only: ReadOnlyState,
    entries: Vec<TranscriptEntry>,
    live_answer: String,
    live_reasoning: String,
    live_reasoning_expanded: bool,
    reasoning_entry_mark: usize,
    expanded: Vec<EntryId>,
    subagent_links: HashMap<u64, EntryId>,
    subagent_runs: HashMap<EntryId, SubagentRow>,
    notices: Vec<Notice>,
    modals: Vec<crate::entities::render_model::modal::ModalView>,
    palette: Option<PaletteState>,
    queue_len: usize,
    shutdown: bool,
}
impl UiState {
    /// Creates an empty UI state for the current terminal size.
    pub(crate) fn new(size: TerminalSize) -> Self {
        Self {
            size,
            model: None,
            mcp: Vec::new(),
            models: Vec::new(),
            skills: Vec::new(),
            goal: GoalView::inactive(),
            context: ContextUsage::new(None, None),
            activity: AgentActivity::Idle,
            read_only: ReadOnlyState::writable(),
            entries: Vec::new(),
            live_answer: String::new(),
            live_reasoning: String::new(),
            live_reasoning_expanded: false,
            reasoning_entry_mark: 0,
            expanded: Vec::new(),
            subagent_links: HashMap::new(),
            subagent_runs: HashMap::new(),
            notices: Vec::new(),
            modals: Vec::new(),
            palette: None,
            queue_len: 0,
            shutdown: false,
        }
    }
    /// Applies one immutable agent event without mutating agent state.
    pub(crate) fn apply(&mut self, event: AgentEvent) {
        if self.apply_subagent(&event) {
            return;
        }
        match event {
            AgentEvent::HistoryLoaded {
                entries,
                skipped_lines,
            } => self.load_history(entries, skipped_lines),
            AgentEvent::TranscriptEntryAdded(entry) => self.add_entry(entry),
            AgentEvent::TranscriptEntryReplaced(entry) => self.replace_entry(entry),
            AgentEvent::AnswerDelta { text, .. } => self.live_answer.push_str(text.as_str()),
            AgentEvent::ReasoningDelta { text, .. } => {
                if self.live_reasoning.is_empty() {
                    self.reasoning_entry_mark = self.entries.len();
                }
                self.live_reasoning.push_str(text.as_str());
            }
            AgentEvent::PermissionRequested(request) => self.push_permission(request),
            AgentEvent::PermissionResolved { correlation_id } => {
                self.remove_modal(correlation_id);
            }
            AgentEvent::QuestionAsked(question) => self.push_question(question),
            AgentEvent::QuestionResolved { correlation_id } => {
                self.remove_modal(correlation_id);
            }
            AgentEvent::ActivityChanged(activity) => {
                self.activity = activity;
                if activity == AgentActivity::Running {
                    self.notices.clear();
                }
            }
            AgentEvent::ContextUsageChanged(usage) => self.context = usage,
            AgentEvent::CompactionStarted { .. } => {
                self.activity = AgentActivity::Compacting;
            }
            AgentEvent::CompactionFinished { compacted, .. } => {
                self.activity = AgentActivity::Idle;
                let level = if compacted {
                    NoticeLevel::Success
                } else {
                    NoticeLevel::Warning
                };
                self.notice(level, "Context compaction finished.");
            }
            AgentEvent::CompactionFailed { error, .. } => {
                self.activity = AgentActivity::Idle;
                self.notice(NoticeLevel::Error, error);
            }
            AgentEvent::ModelChanged(model) => self.model = Some(model.to_string()),
            AgentEvent::McpStatusChanged(statuses) => self.mcp = statuses,
            AgentEvent::QueueChanged(queue) => self.queue_len = queue.len(),
            AgentEvent::GoalChanged(goal) => self.goal = goal,
            AgentEvent::Notice(notice) => self.push_notice(notice),
            AgentEvent::ReadOnlyChanged(state) => self.read_only = state,
            AgentEvent::ShutdownComplete => self.shutdown = true,
            AgentEvent::SubagentStarted { .. }
            | AgentEvent::SubagentActivity { .. }
            | AgentEvent::SubagentFinished { .. } => {}
        }
    }
    /// Reports whether the agent completed its shutdown sequence.
    pub(crate) const fn shutdown(&self) -> bool {
        self.shutdown
    }
    /// Reports whether the terminal is too small for the normal UI.
    pub(crate) fn is_fallback(&self) -> bool {
        crate::entities::render_model::layout::LayoutView::new(self.size).fallback
    }
    /// Updates the terminal size.
    pub(crate) fn resize(&mut self, size: TerminalSize) {
        self.size = size;
    }
    /// Returns the active modal, if any.
    pub(crate) fn active_modal(&self) -> Option<&crate::entities::render_model::modal::ModalView> {
        self.modals.last()
    }
    /// Returns the active modal for mutation.
    pub(crate) fn active_modal_mut(
        &mut self,
    ) -> Option<&mut crate::entities::render_model::modal::ModalView> {
        self.modals.last_mut()
    }
    /// Pushes one modal onto the overlay stack.
    pub(crate) fn push_modal(&mut self, modal: crate::entities::render_model::modal::ModalView) {
        if let Some(id) = modal.correlation_id()
            && self
                .modals
                .iter()
                .any(|item| item.correlation_id() == Some(id))
        {
            return;
        }
        self.modals.push(modal);
    }
    /// Removes the top modal without resolving it.
    pub(crate) fn pop_modal(&mut self) -> Option<crate::entities::render_model::modal::ModalView> {
        self.modals.pop()
    }
    /// Returns the palette state for mutation.
    pub(crate) fn palette_mut(&mut self) -> Option<&mut PaletteState> {
        self.palette.as_mut()
    }
    /// Returns the palette state.
    pub(crate) fn palette(&self) -> Option<&PaletteState> {
        self.palette.as_ref()
    }
    /// Opens or replaces the palette.
    pub(crate) fn set_palette(&mut self, palette: Option<PaletteState>) {
        self.palette = palette;
    }
    /// Toggles the newest collapsible block.
    pub(crate) fn toggle_last_block(&mut self) {
        if !self.live_reasoning.is_empty() && self.reasoning_entry_mark == self.entries.len() {
            self.live_reasoning_expanded = !self.live_reasoning_expanded;
            return;
        }
        let Some(id) = self
            .entries
            .iter()
            .rev()
            .find(|entry| is_collapsible(&entry.kind))
            .map(|entry| entry.id)
        else {
            return;
        };
        if let Some(index) = self.expanded.iter().position(|item| *item == id) {
            self.expanded.remove(index);
        } else {
            self.expanded.push(id);
        }
    }
    /// Clears transient notices before a new submission.
    pub(crate) fn clear_notices(&mut self) {
        self.notices.clear();
    }
    /// Appends one transient notice, keeping only the newest three.
    pub(crate) fn notice(&mut self, level: NoticeLevel, text: impl Into<String>) {
        self.push_notice(Notice::new(level, text));
    }
    fn load_history(&mut self, entries: Vec<TranscriptEntry>, skipped_lines: usize) {
        self.entries = entries;
        self.live_answer.clear();
        self.live_reasoning.clear();
        self.live_reasoning_expanded = false;
        self.reasoning_entry_mark = 0;
        self.expanded.clear();
        self.subagent_links.clear();
        self.subagent_runs.clear();
        if skipped_lines > 0 {
            self.notice(
                NoticeLevel::Warning,
                format!("{skipped_lines} unreadable history lines were skipped."),
            );
        }
    }
    fn add_entry(&mut self, mut entry: TranscriptEntry) {
        if let TranscriptEntryKind::Assistant { reasoning, .. } = &mut entry.kind
            && reasoning.is_none()
            && !self.live_reasoning.is_empty()
        {
            *reasoning = Some(self.live_reasoning.clone());
        }
        if matches!(entry.kind, TranscriptEntryKind::Assistant { .. }) {
            if self.live_reasoning_expanded && !self.expanded.contains(&entry.id) {
                self.expanded.push(entry.id);
            }
            self.live_answer.clear();
            self.live_reasoning.clear();
            self.live_reasoning_expanded = false;
            self.reasoning_entry_mark = 0;
        }
        if let Some(index) = self.entries.iter().position(|item| item.id == entry.id) {
            self.entries[index] = entry;
        } else {
            self.entries.push(entry);
        }
    }
    fn replace_entry(&mut self, entry: TranscriptEntry) {
        if matches!(entry.kind, TranscriptEntryKind::ToolResult { .. }) {
            self.entries.push(entry);
            return;
        }
        if let Some(index) = self.entries.iter().position(|item| item.id == entry.id) {
            self.entries[index] = entry;
        } else {
            self.entries.push(entry);
        }
    }
    fn push_permission(&mut self, request: PermissionRequestView) {
        self.push_modal(crate::entities::render_model::modal::ModalView::permission(
            request,
        ));
    }
    fn push_question(&mut self, question: QuestionView) {
        self.push_modal(crate::entities::render_model::modal::ModalView::question(
            question,
        ));
    }
    fn remove_modal(&mut self, correlation_id: u64) {
        self.modals
            .retain(|modal| modal.correlation_id() != Some(correlation_id));
    }
    fn push_notice(&mut self, notice: Notice) {
        self.notices.push(notice);
        while self.notices.len() > 3 {
            self.notices.remove(0);
        }
    }
}
fn is_collapsible(kind: &TranscriptEntryKind) -> bool {
    matches!(
        kind,
        TranscriptEntryKind::ToolCall { .. }
            | TranscriptEntryKind::ToolResult { .. }
            | TranscriptEntryKind::Summary { .. }
            | TranscriptEntryKind::Assistant {
                reasoning: Some(_),
                ..
            }
    )
}
