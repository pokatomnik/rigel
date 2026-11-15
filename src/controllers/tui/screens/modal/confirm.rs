use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::Style,
    widgets::{Paragraph, Wrap},
};

use crate::entities::{
    render_model::modal::ExitChoice,
    theme::{palette::Palette, token::ColorToken},
};

use super::{color, hint, modal_block, option_line};

const EXIT_HINT: &str = "↑/↓ j/k · Enter · Esc";

pub(super) fn render_exit(
    frame: &mut Frame<'_>,
    area: Rect,
    selected: ExitChoice,
    palette: &Palette,
) {
    render_confirm(
        frame,
        area,
        "Exit Rigel?",
        "The agent will stop at a safe boundary before the process exits.",
        "Exit",
        selected,
        palette,
    );
}

pub(super) fn render_new(
    frame: &mut Frame<'_>,
    area: Rect,
    selected: ExitChoice,
    palette: &Palette,
) {
    render_confirm(
        frame,
        area,
        "Start a new conversation?",
        "This deletes the current history, including compacted archives.",
        "Delete",
        selected,
        palette,
    );
}

fn render_confirm(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    message: &str,
    confirm_label: &str,
    selected: ExitChoice,
    palette: &Palette,
) {
    let block = modal_block(title, ColorToken::Warning, palette);
    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(block.inner(area));
    frame.render_widget(block, area);
    frame.render_widget(
        Paragraph::new(message)
            .style(Style::default().fg(color(palette, ColorToken::Muted)))
            .wrap(Wrap { trim: true }),
        chunks[0],
    );
    for (index, (choice, label)) in [
        (ExitChoice::Cancel, "Cancel"),
        (ExitChoice::Exit, confirm_label),
    ]
    .into_iter()
    .enumerate()
    {
        frame.render_widget(
            Paragraph::new(option_line(label, choice == selected, palette)),
            Rect {
                x: chunks[1].x,
                y: chunks[1].y + index as u16,
                width: chunks[1].width,
                height: 1,
            },
        );
    }
    frame.render_widget(hint(EXIT_HINT, palette), chunks[3]);
}
