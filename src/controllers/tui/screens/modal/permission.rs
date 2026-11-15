use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::Style,
    widgets::{Paragraph, Wrap},
};

use crate::entities::{
    agent_event::payload::PermissionRequestView,
    theme::{palette::Palette, token::ColorToken},
    tool_confirm_result::outcome::ToolConfirmResult,
};

use super::{color, hint, modal_block, option_line};

pub(super) fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    request: &PermissionRequestView,
    selected: usize,
    scroll: u16,
    palette: &Palette,
) {
    let title = format!("Permission required · {}", request.tool_name);
    let block = modal_block(title.as_str(), ColorToken::Warning, palette);
    let inner = block.inner(area);
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(2),
        Constraint::Length(5),
        Constraint::Length(1),
    ])
    .split(inner);
    frame.render_widget(block, area);
    let origin = match request.origin {
        crate::entities::agent_event::payload::InteractionOrigin::MainAgent => "agent",
        crate::entities::agent_event::payload::InteractionOrigin::Subagent => "subagent",
    };
    frame.render_widget(
        Paragraph::new(format!(
            "{} · scope: {origin} · arguments:",
            request.tool_name
        ))
        .style(Style::default().fg(color(palette, ColorToken::Muted))),
        chunks[0],
    );
    let arguments = crate::controllers::tui::json::lines(&request.arguments, palette);
    frame.render_widget(
        Paragraph::new(arguments)
            .scroll((scroll, 0))
            .wrap(Wrap { trim: false }),
        chunks[1],
    );
    for (index, decision) in ToolConfirmResult::ALL.iter().enumerate() {
        let line = option_line(decision.to_string().as_str(), index == selected, palette);
        frame.render_widget(
            Paragraph::new(line),
            Rect {
                x: chunks[2].x,
                y: chunks[2].y + index as u16,
                width: chunks[2].width,
                height: 1,
            },
        );
    }
    frame.render_widget(
        hint("↑/↓ j/k · PgUp/PgDn · Enter · Esc", palette),
        chunks[3],
    );
}
