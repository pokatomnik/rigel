use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::Style,
    widgets::{Paragraph, Wrap},
};

use crate::entities::{
    agent_event::payload::QuestionView,
    theme::{palette::Palette, token::ColorToken},
};

use super::{color, hint, modal_block, option_line};
use crate::controllers::tui::screens::visible_offset;

pub(super) fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    request: &QuestionView,
    selected: usize,
    input: &str,
    palette: &Palette,
) {
    let block = modal_block("Question", ColorToken::Accent, palette);
    let inner = block.inner(area);
    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(inner);
    frame.render_widget(block, area);
    frame.render_widget(
        Paragraph::new(request.question.as_str())
            .style(Style::default().fg(color(palette, ColorToken::Text)))
            .wrap(Wrap { trim: true }),
        chunks[0],
    );
    let free_form = request.answers.is_empty() || selected >= request.answers.len();
    if free_form {
        frame.render_widget(
            Paragraph::new(format!("> {input}"))
                .style(Style::default().fg(color(palette, ColorToken::Accent))),
            chunks[1],
        );
    } else {
        let mut lines = request
            .answers
            .iter()
            .enumerate()
            .map(|(index, answer)| option_line(answer.as_str(), index == selected, palette))
            .collect::<Vec<_>>();
        lines.push(option_line(
            "Something else",
            selected == request.answers.len(),
            palette,
        ));
        let visible = usize::from(chunks[1].height);
        let offset = visible_offset(lines.len(), selected, visible);
        let visible_lines = lines
            .into_iter()
            .skip(offset)
            .take(visible)
            .collect::<Vec<_>>();
        frame.render_widget(Paragraph::new(visible_lines), chunks[1]);
    }
    let hint_text = if free_form {
        "Enter submit · Esc cancel"
    } else {
        "↑/↓ j/k · Enter · Esc"
    };
    frame.render_widget(hint(hint_text, palette), chunks[3]);
}
