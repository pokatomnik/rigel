use ratatui::{
    Frame, Terminal,
    backend::Backend,
    layout::{Constraint, Direction, Layout},
    style::Style,
};

use crate::entities::{
    application::input_reaction::ScrollIntent,
    render_model::{modal::ModalView, model::RenderModel, transcript::TranscriptScroll},
    theme::{palette::Palette, token::ColorToken},
};

use super::{
    composer::Composer,
    screens::{Presentation, chat, fallback, header, modal, palette, status},
    theme,
};

/// Renders immutable presentation snapshots through one Ratatui terminal.
pub(crate) struct Renderer<B: Backend> {
    terminal: Terminal<B>,
    palette: Palette,
    scroll: TranscriptScroll,
}

impl<B: Backend> Renderer<B> {
    /// Creates a renderer for one terminal and one fixed theme.
    pub(crate) fn new(terminal: Terminal<B>, palette: Palette) -> Self {
        Self {
            terminal,
            palette,
            scroll: TranscriptScroll::new(),
        }
    }

    /// Queues one transcript viewport movement.
    pub(crate) fn scroll(&mut self, intent: ScrollIntent) {
        self.scroll.request(intent);
    }

    /// Draws one frame and reports terminal failures.
    pub(crate) fn draw(
        &mut self,
        model: &RenderModel,
        composer: &mut Composer,
    ) -> anyhow::Result<()> {
        let palette = &self.palette;
        let scroll = &mut self.scroll;
        self.terminal
            .draw(|frame| render_frame(frame, model, composer, palette, scroll))
            .map_err(|error| anyhow::anyhow!("terminal draw failed: {error}"))?;
        Ok(())
    }

    /// Returns the backend for TestBackend assertions.
    #[cfg(test)]
    pub(crate) fn backend(&self) -> &B {
        self.terminal.backend()
    }
}

fn render_frame(
    frame: &mut Frame<'_>,
    model: &RenderModel,
    composer: &mut Composer,
    palette: &Palette,
    scroll: &mut TranscriptScroll,
) {
    let presentation = Presentation { model, palette };
    let area = frame.area();
    frame.buffer_mut().set_style(
        area,
        Style::default()
            .fg(theme::color(palette.color(ColorToken::Text)))
            .bg(theme::color(palette.color(ColorToken::Background))),
    );
    if model.layout.fallback {
        fallback::render(frame, area, &presentation);
        if matches!(model.modal, Some(ModalView::ConfirmExit { .. })) {
            modal::render_fallback_exit(frame, area, palette);
        }
        return;
    }
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(area);
    header::render(frame, chunks[0], &presentation);
    chat::render(frame, chunks[1], &presentation, composer, scroll);
    status::render(frame, chunks[2], &presentation);
    if model.palette.is_some() {
        palette::render(frame, area, &presentation);
    }
    if let Some(modal_view) = model.modal.as_ref() {
        modal::render(frame, area, modal_view, palette);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend, buffer::Buffer, style::Color};

    use super::Renderer;
    use crate::entities::render_model::model::RenderModel;
    use crate::{
        controllers::tui::composer::Composer,
        entities::{
            application::input_event::TerminalSize,
            conversation::context_usage::ContextUsage,
            render_model::{
                transcript::{ToolResultRow, ToolRow, TranscriptRow, TranscriptView},
                views::{ComposerView, HeaderView, McpSummary},
            },
            theme::palette::Palette,
            transcript::entry::EntryId,
        },
    };

    fn render(model: &RenderModel) -> anyhow::Result<Buffer> {
        let backend = TestBackend::new(model.layout.size.width, model.layout.size.height);
        let terminal = Terminal::new(backend)?;
        let mut renderer = Renderer::new(terminal, Palette::rigel_night());
        let mut composer = Composer::new();
        renderer.draw(model, &mut composer)?;
        Ok(renderer.backend().buffer().clone())
    }

    fn buffer_text(buffer: &Buffer) -> String {
        let mut text = String::new();
        for row in 0..buffer.area.height {
            for column in 0..buffer.area.width {
                text.push_str(buffer[(column, row)].symbol());
            }
            text.push('\n');
        }
        text
    }

    fn normal_model() -> RenderModel {
        let id = EntryId::new(1);
        RenderModel::empty(TerminalSize::new(80, 24))
            .with_header(
                HeaderView::new(
                    Some("model-a".to_string()),
                    ContextUsage::new(Some(42_000), Some(128_000)),
                )
                .with_mcp(McpSummary::new(3, 3)),
            )
            .with_transcript(TranscriptView::new(
                vec![TranscriptRow::User {
                    id,
                    text: "hello".to_string(),
                }],
                Vec::new(),
            ))
            .with_composer(ComposerView::new("draft".to_string()))
    }

    #[test]
    fn small_terminal_shows_only_the_english_fallback() -> anyhow::Result<()> {
        let buffer = render(&RenderModel::empty(TerminalSize::new(59, 20)))?;
        let text = buffer_text(&buffer);

        assert!(text.contains("Terminal too small to display the UI."));
        assert!(!text.contains("Rigel"));
        assert!(!text.contains("Message"));
        Ok(())
    }

    #[test]
    fn palette_viewport_keeps_the_selected_item_visible() -> anyhow::Result<()> {
        use crate::entities::{
            palette::{
                item::{PaletteAction, PaletteItem},
                level::PaletteLevel,
            },
            render_model::palette::PaletteView,
        };

        let mut items = vec![PaletteItem::new("back", "← Back", PaletteAction::Back)];
        items.extend((0..16).map(|index| {
            PaletteItem::new(
                format!("model-{index}"),
                format!("model-{index}"),
                PaletteAction::Info,
            )
        }));
        let view = PaletteView::new(PaletteLevel::Models, items);
        let last = RenderModel::empty(TerminalSize::new(60, 20))
            .with_palette(Some(view.clone().with_selected(16)));
        let last_text = buffer_text(&render(&last)?);
        assert!(last_text.contains("› model-15"));

        let first =
            RenderModel::empty(TerminalSize::new(60, 20)).with_palette(Some(view.with_selected(0)));
        let first_text = buffer_text(&render(&first)?);
        assert!(first_text.contains("› ← Back"));
        Ok(())
    }

    #[test]
    fn minimum_size_shows_the_normal_ui() -> anyhow::Result<()> {
        let model = RenderModel::empty(TerminalSize::new(60, 20));
        let text = buffer_text(&render(&model)?);

        assert!(text.contains("Rigel"));
        assert!(text.contains("Message"));
        assert!(!text.contains("Terminal too small"));
        Ok(())
    }

    #[test]
    fn normal_frame_contains_header_transcript_and_composer() -> anyhow::Result<()> {
        let text = buffer_text(&render(&normal_model())?);

        assert!(text.contains("Rigel"));
        assert!(text.contains("model-a"));
        assert!(text.contains("MCP 3/3"));
        assert!(text.contains("context 42K/128K"));
        assert!(text.contains("You"));
        assert!(text.contains("hello"));
        assert!(text.contains("Message"));
        Ok(())
    }

    #[test]
    fn every_buffer_cell_has_explicit_rgb_colors() -> anyhow::Result<()> {
        let buffer = render(&normal_model())?;

        for cell in buffer.content() {
            assert!(matches!(cell.fg, Color::Rgb(_, _, _)));
            assert!(matches!(cell.bg, Color::Rgb(_, _, _)));
        }
        Ok(())
    }

    #[test]
    fn collapsed_tool_hides_arguments_and_expanded_tool_shows_full_payload() -> anyhow::Result<()> {
        let id = EntryId::new(8);
        let tool = ToolRow::new(
            id,
            "read_file".to_string(),
            "{\n  \"path\": \"src/main.rs\"\n}".to_string(),
        )
        .with_result(ToolResultRow::new("file contents".to_string(), false));
        let collapsed = RenderModel::empty(TerminalSize::new(80, 24)).with_transcript(
            TranscriptView::new(vec![TranscriptRow::Tool(tool.clone())], Vec::new()),
        );
        let collapsed_text = buffer_text(&render(&collapsed)?);
        assert!(collapsed_text.contains("[read_file]"));
        assert!(!collapsed_text.contains("src/main.rs"));
        assert!(!collapsed_text.contains("file contents"));

        let expanded = collapsed.with_transcript(TranscriptView::new(
            vec![TranscriptRow::Tool(tool)],
            vec![id],
        ));
        let expanded_text = buffer_text(&render(&expanded)?);
        assert!(expanded_text.contains("src/main.rs"));
        assert!(expanded_text.contains("file contents"));
        Ok(())
    }

    #[test]
    fn subagent_block_hides_child_work_until_expanded() -> anyhow::Result<()> {
        let id = EntryId::new(9);
        let subagent = crate::entities::render_model::transcript::SubagentRow {
            prompt: "inspect the repository".to_string(),
            activity: "[read_file]".to_string(),
            report: Some("done".to_string()),
        };
        let tool = ToolRow::new(id, "spawn_subagent".to_string(), "{}".to_string())
            .with_subagent(subagent);
        let model = RenderModel::empty(TerminalSize::new(80, 24)).with_transcript(
            TranscriptView::new(vec![TranscriptRow::Tool(tool.clone())], Vec::new()),
        );
        let collapsed = buffer_text(&render(&model)?);
        assert!(collapsed.contains("[spawn_subagent]"));
        assert!(collapsed.contains("inspect the repository"));
        assert!(!collapsed.contains("[read_file]"));
        assert!(!collapsed.contains("done"));

        let expanded = model.with_transcript(TranscriptView::new(
            vec![TranscriptRow::Tool(tool)],
            vec![id],
        ));
        let expanded_text = buffer_text(&render(&expanded)?);
        assert!(expanded_text.contains("Task"));
        assert!(expanded_text.contains("[read_file]"));
        assert!(expanded_text.contains("Report"));
        Ok(())
    }

    #[test]
    fn scrolling_up_keeps_the_reading_anchor_when_new_rows_arrive() -> anyhow::Result<()> {
        let backend = TestBackend::new(80, 24);
        let terminal = Terminal::new(backend)?;
        let mut renderer = Renderer::new(terminal, Palette::rigel_night());
        let mut composer = Composer::new();
        renderer.draw(&rows_model(0..40), &mut composer)?;
        renderer.scroll(crate::entities::application::input_reaction::ScrollIntent::PageUp);
        renderer.draw(&rows_model(0..40), &mut composer)?;
        let scrolled = buffer_text(renderer.backend().buffer());
        assert!(scrolled.contains("message-3"));
        assert!(!scrolled.contains("message-39"));

        renderer.draw(&rows_model(0..60), &mut composer)?;
        let after_growth = buffer_text(renderer.backend().buffer());
        assert!(after_growth.contains("message-3"));
        assert!(!after_growth.contains("message-59"));
        Ok(())
    }

    fn rows_model(range: std::ops::Range<u64>) -> RenderModel {
        let rows = range
            .map(|index| TranscriptRow::User {
                id: EntryId::new(index),
                text: format!("message-{index}"),
            })
            .collect::<Vec<_>>();
        RenderModel::empty(TerminalSize::new(80, 24))
            .with_transcript(TranscriptView::new(rows, Vec::new()))
    }

    #[test]
    fn model_palette_shows_the_configured_context_size() -> anyhow::Result<()> {
        use crate::entities::{
            palette::{
                item::{PaletteAction, PaletteItem},
                level::PaletteLevel,
            },
            render_model::palette::PaletteView,
        };
        let view = PaletteView::new(
            PaletteLevel::Models,
            vec![
                PaletteItem::new("model-a", "model-a", PaletteAction::Info)
                    .with_detail("32K")
                    .with_right_detail(),
                PaletteItem::new("back", "← Back", PaletteAction::Back),
            ],
        );
        let model = RenderModel::empty(TerminalSize::new(80, 24)).with_palette(Some(view));
        let text = buffer_text(&render(&model)?);

        assert!(text.contains("Commands · Models"));
        assert!(text.contains("32K"));
        assert!(text.contains("← Back"));
        Ok(())
    }

    #[test]
    fn goal_modal_shows_input_error_and_shortcut_hint() -> anyhow::Result<()> {
        let modal = crate::entities::render_model::modal::ModalView::Goal {
            input: "finish the task".to_string(),
            error: true,
        };
        let text = buffer_text(&render(&normal_model().with_modal(Some(modal)))?);

        assert!(text.contains("Set a goal"));
        assert!(text.contains("finish the task"));
        assert!(text.contains("Goal cannot be empty."));
        assert!(text.contains("Enter: start"));
        assert!(text.contains("Alt+Enter: line"));
        Ok(())
    }

    #[test]
    fn new_conversation_confirmation_offers_delete() -> anyhow::Result<()> {
        let modal = crate::entities::render_model::modal::ModalView::confirm_new();
        let text = buffer_text(&render(&normal_model().with_modal(Some(modal)))?);

        assert!(text.contains("Start a new conversation?"));
        assert!(text.contains("Delete"));
        assert!(text.contains("Cancel"));
        Ok(())
    }

    #[test]
    fn permission_modal_shows_the_tool_name_and_choices() -> anyhow::Result<()> {
        let modal = crate::entities::render_model::modal::ModalView::permission(
            crate::entities::agent_event::payload::PermissionRequestView::new(
                7,
                "run_command".to_string(),
                serde_json::json!({"command": "pwd"}),
                crate::entities::agent_event::payload::InteractionOrigin::MainAgent,
            ),
        );
        let text = buffer_text(&render(&normal_model().with_modal(Some(modal)))?);

        assert!(text.contains("Permission required"));
        assert!(text.contains("run_command"));
        assert!(text.contains("Deny"));
        assert!(text.contains("Allow once"));
        Ok(())
    }

    #[test]
    fn follow_tail_shows_the_newest_rows() -> anyhow::Result<()> {
        let rows = (0..40)
            .map(|index| TranscriptRow::User {
                id: EntryId::new(index),
                text: format!("message-{index}"),
            })
            .collect::<Vec<_>>();
        let model = RenderModel::empty(TerminalSize::new(80, 24))
            .with_transcript(TranscriptView::new(rows, Vec::new()));
        let text = buffer_text(&render(&model)?);

        assert!(text.contains("message-39"));
        assert!(!text.contains("message-0"));
        Ok(())
    }
}
