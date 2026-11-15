use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::Style,
    widgets::{Paragraph, Wrap},
};

use crate::entities::theme::{palette::Palette, token::ColorToken};

use super::{color, hint, modal_block};

pub(super) fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    input: &str,
    error: bool,
    palette: &Palette,
) {
    let block = modal_block("Set a goal", ColorToken::Accent, palette);
    let chunks = Layout::vertical([
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(block.inner(area));
    frame.render_widget(block, area);
    let paragraph = Paragraph::new(format!("{input}▌"))
        .style(Style::default().fg(color(palette, ColorToken::Text)))
        .wrap(Wrap { trim: false });
    let total = paragraph.line_count(chunks[0].width);
    let offset = total.saturating_sub(chunks[0].height as usize) as u16;
    frame.render_widget(paragraph.scroll((offset, 0)), chunks[0]);
    if error {
        frame.render_widget(
            Paragraph::new("Goal cannot be empty.")
                .style(Style::default().fg(color(palette, ColorToken::Error))),
            chunks[1],
        );
    }
    let shortcut = "Enter: start · Alt+Enter: line · Esc: cancel";
    frame.render_widget(hint(shortcut, palette), chunks[2]);
}
