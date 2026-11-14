use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders},
};

use crate::{
    controllers::tui::{composer::Composer, theme},
    entities::{
        render_model::transcript::TranscriptScroll,
        theme::{palette::Palette, token::ColorToken},
    },
};

use super::{Presentation, transcript};

/// Renders the transcript and composer area.
pub(crate) fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    presentation: &Presentation<'_>,
    composer: &mut Composer,
    scroll: &mut TranscriptScroll,
) {
    let composer_height = (composer.line_count() as u16 + 2).clamp(3, 8);
    let chunks =
        Layout::vertical([Constraint::Min(1), Constraint::Length(composer_height)]).split(area);
    transcript::render(frame, chunks[0], presentation, scroll);
    render_composer(frame, chunks[1], presentation, composer);
}

fn render_composer(
    frame: &mut Frame<'_>,
    area: Rect,
    presentation: &Presentation<'_>,
    composer: &mut Composer,
) {
    let palette = presentation.palette;
    let view = &presentation.model.composer;
    let border = if view.submit_focused {
        ColorToken::Success
    } else if view.focused {
        ColorToken::Accent
    } else {
        ColorToken::Border
    };
    let title = Line::from(vec![
        Span::styled(
            " Message ",
            Style::default().fg(color(palette, ColorToken::Muted)),
        ),
        Span::styled(
            "[ Send ]",
            Style::default()
                .fg(color(
                    palette,
                    if view.submit_focused {
                        ColorToken::Success
                    } else {
                        ColorToken::Accent
                    },
                ))
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color(palette, border)))
        .title(title)
        .style(Style::default().bg(color(palette, ColorToken::Surface)));
    let textarea = composer.textarea_mut();
    textarea.set_block(block);
    textarea.set_style(
        Style::default()
            .fg(color(palette, ColorToken::Text))
            .bg(color(palette, ColorToken::Surface)),
    );
    textarea.set_cursor_style(Style::default().fg(color(palette, ColorToken::Accent)));
    textarea.set_placeholder_style(Style::default().fg(color(palette, ColorToken::Muted)));
    frame.render_widget(composer.textarea(), area);
    if view.focused
        && let Some(position) = composer.cursor_position(area)
    {
        frame.set_cursor_position(ratatui::layout::Position::new(position.0, position.1));
    }
}

fn color(palette: &Palette, token: ColorToken) -> ratatui::style::Color {
    theme::token(palette, token)
}
