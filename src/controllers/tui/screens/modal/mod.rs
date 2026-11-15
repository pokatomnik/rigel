mod confirm;
mod goal;
mod permission;
mod question;

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::{
    controllers::tui::theme,
    entities::{
        render_model::modal::ModalView,
        theme::{palette::Palette, token::ColorToken},
    },
};

/// Renders the active overlay inside the central half of the screen.
pub(crate) fn render(frame: &mut Frame<'_>, area: Rect, modal: &ModalView, palette: &Palette) {
    let popup = popup_area(area);
    frame.render_widget(Clear, popup);
    match modal {
        ModalView::ConfirmExit { selected } => {
            confirm::render_exit(frame, popup, *selected, palette);
        }
        ModalView::ConfirmNew { selected } => {
            confirm::render_new(frame, popup, *selected, palette);
        }
        ModalView::Permission {
            request,
            selected,
            scroll,
        } => permission::render(frame, popup, request, *selected, *scroll, palette),
        ModalView::Question {
            request,
            selected,
            input,
        } => question::render(frame, popup, request, *selected, input, palette),
        ModalView::Goal { input, error } => goal::render(frame, popup, input, *error, palette),
    }
}

/// Renders the exit confirmation inside the tiny-terminal fallback.
pub(crate) fn render_fallback_exit(frame: &mut Frame<'_>, area: Rect, palette: &Palette) {
    let popup = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(3) / 2,
        width: area.width,
        height: 3.min(area.height),
    };
    let text = Line::from(vec![
        Span::styled(
            "Exit? ",
            Style::default().fg(color(palette, ColorToken::Text)),
        ),
        Span::styled(
            "y",
            Style::default()
                .fg(color(palette, ColorToken::Success))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            " / ",
            Style::default().fg(color(palette, ColorToken::Muted)),
        ),
        Span::styled(
            "n",
            Style::default()
                .fg(color(palette, ColorToken::Error))
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(text)
            .alignment(ratatui::layout::Alignment::Center)
            .style(Style::default().bg(color(palette, ColorToken::Background))),
        popup,
    );
}

fn popup_area(area: Rect) -> Rect {
    let width = area
        .width
        .saturating_mul(4)
        .saturating_div(5)
        .max(20)
        .min(area.width);
    let height = (area.height / 2).max(6).min(area.height);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

pub(super) fn modal_block(title: &str, token: ColorToken, palette: &Palette) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color(palette, token)))
        .title(Span::styled(
            format!(" {title} "),
            Style::default()
                .fg(color(palette, token))
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(color(palette, ColorToken::Surface)))
}

pub(super) fn option_line(label: &str, selected: bool, palette: &Palette) -> Line<'static> {
    let style = if selected {
        Style::default()
            .fg(color(palette, ColorToken::Background))
            .bg(color(palette, ColorToken::Accent))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(color(palette, ColorToken::Text))
    };
    Line::from(Span::styled(format!(" {label} "), style))
}

pub(super) fn hint(text: &str, palette: &Palette) -> Paragraph<'static> {
    Paragraph::new(Span::styled(
        text.to_string(),
        Style::default().fg(color(palette, ColorToken::Muted)),
    ))
}

pub(super) fn color(palette: &Palette, token: ColorToken) -> Color {
    theme::token(palette, token)
}
