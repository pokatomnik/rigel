use crate::entities::{
    agent_event::status::{AgentActivity, ReadOnlyState},
    application::input_event::TerminalSize,
    conversation::context_usage::ContextUsage,
    render_model::{
        layout::LayoutView,
        modal::ModalView,
        palette::PaletteView,
        transcript::TranscriptView,
        views::{ComposerView, HeaderView, StatusView},
    },
};

/// One immutable presentation snapshot rendered by the terminal adapter.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RenderModel {
    /// Terminal size and the resulting fallback decision.
    pub(crate) layout: LayoutView,
    /// Header with model, MCP, and context information.
    pub(crate) header: HeaderView,
    /// Transcript rows and presentation-only expansion state.
    pub(crate) transcript: TranscriptView,
    /// Composer text and focus state.
    pub(crate) composer: ComposerView,
    /// Status line, notices, and read-only state.
    pub(crate) status: StatusView,
    /// Active overlay that owns keyboard input.
    pub(crate) modal: Option<ModalView>,
    /// Command palette overlay, if open.
    pub(crate) palette: Option<PaletteView>,
}

impl RenderModel {
    /// Creates an empty frame for the given terminal size.
    pub(crate) fn empty(size: TerminalSize) -> Self {
        Self {
            layout: LayoutView::new(size),
            header: HeaderView::new(None, ContextUsage::new(None, None)),
            transcript: TranscriptView::default(),
            composer: ComposerView::default(),
            status: StatusView::new(AgentActivity::Idle, ReadOnlyState::writable()),
            modal: None,
            palette: None,
        }
    }

    /// Replaces the header for this frame.
    pub(crate) fn with_header(mut self, header: HeaderView) -> Self {
        self.header = header;
        self
    }

    /// Replaces the transcript for this frame.
    pub(crate) fn with_transcript(mut self, transcript: TranscriptView) -> Self {
        self.transcript = transcript;
        self
    }

    /// Replaces the composer for this frame.
    pub(crate) fn with_composer(mut self, composer: ComposerView) -> Self {
        self.composer = composer;
        self
    }

    /// Replaces the status line for this frame.
    pub(crate) fn with_status(mut self, status: StatusView) -> Self {
        self.status = status;
        self
    }

    /// Replaces the active modal for this frame.
    pub(crate) fn with_modal(mut self, modal: Option<ModalView>) -> Self {
        self.modal = modal;
        self
    }

    /// Replaces the command palette for this frame.
    pub(crate) fn with_palette(mut self, palette: Option<PaletteView>) -> Self {
        self.palette = palette;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::RenderModel;
    use crate::entities::{
        agent_event::status::{AgentActivity, ReadOnlyState},
        application::input_event::TerminalSize,
        conversation::context_usage::ContextUsage,
        render_model::{
            transcript::{TranscriptRow, TranscriptView},
            views::{ComposerView, HeaderView, McpSummary, StatusView},
        },
        transcript::entry::EntryId,
    };

    #[test]
    fn empty_model_has_a_usable_status_and_layout() {
        let model = RenderModel::empty(TerminalSize::new(80, 24));

        assert!(!model.layout.fallback);
        assert!(model.transcript.rows.is_empty());
        assert_eq!(model.status.activity, AgentActivity::Idle);
    }

    #[test]
    fn builders_replace_each_section_without_touching_others() {
        let model = RenderModel::empty(TerminalSize::new(80, 24))
            .with_header(
                HeaderView::new(
                    Some("model-a".to_string()),
                    ContextUsage::new(Some(1), Some(10)),
                )
                .with_mcp(McpSummary::new(1, 2))
                .with_activity(AgentActivity::Running),
            )
            .with_composer(ComposerView::new("hello".to_string()))
            .with_transcript(TranscriptView::new(
                vec![TranscriptRow::User {
                    id: EntryId::new(1),
                    text: "hello".to_string(),
                }],
                vec![EntryId::new(1)],
            ))
            .with_status(StatusView::new(
                AgentActivity::Stopping,
                ReadOnlyState::writable(),
            ));

        assert_eq!(model.header.mcp, McpSummary::new(1, 2));
        assert_eq!(model.composer.text, "hello");
        assert!(model.transcript.is_expanded(EntryId::new(1)));
        assert_eq!(model.status.activity, AgentActivity::Stopping);
    }
}
