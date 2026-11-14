use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::Style,
    widgets::{Paragraph, Wrap},
};

use crate::entities::theme::{palette::Palette, token::ColorToken};

use super::Presentation;

/// English warning shown when the terminal is too small for the normal UI.
pub(crate) const TOO_SMALL_MESSAGE: &str =
    "Terminal too small to display the UI. Resize the terminal window.";

/// Renders the centered fallback without any normal UI elements.
pub(crate) fn render(frame: &mut Frame<'_>, area: Rect, presentation: &Presentation<'_>) {
    let palette = presentation.palette;
    let rows = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(2),
        Constraint::Fill(1),
    ])
    .split(area);
    let paragraph = Paragraph::new(TOO_SMALL_MESSAGE)
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true })
        .style(
            Style::default()
                .fg(color(palette, ColorToken::Text))
                .bg(color(palette, ColorToken::Background)),
        );
    frame.render_widget(paragraph, rows[1]);
}

fn color(palette: &Palette, token: ColorToken) -> ratatui::style::Color {
    crate::controllers::tui::theme::token(palette, token)
}

#[cfg(test)]
mod tests {
    use super::TOO_SMALL_MESSAGE;

    #[test]
    fn fallback_message_is_the_approved_english_copy() {
        assert_eq!(
            TOO_SMALL_MESSAGE,
            "Terminal too small to display the UI. Resize the terminal window."
        );
    }
}
