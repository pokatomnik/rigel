use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::{
    controllers::tui::theme,
    entities::{
        palette::{item::ItemTone, level::PaletteLevel},
        render_model::palette::PaletteView,
        theme::{palette::Palette, token::ColorToken},
    },
};

/// Renders the command palette as a centered list.
pub(crate) fn render(frame: &mut Frame<'_>, area: Rect, view: &PaletteView, palette: &Palette) {
    let height = (view.items.len() as u16 + 4)
        .max(5)
        .min(area.height.saturating_mul(3) / 4);
    let width = area.width.saturating_mul(3) / 5;
    let popup = Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color(palette, ColorToken::Border)))
        .title(Span::styled(
            format!(" {} ", palette_title(view)),
            Style::default()
                .fg(color(palette, ColorToken::Accent))
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(color(palette, ColorToken::Surface)));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(inner);
    let lines = view
        .items
        .iter()
        .enumerate()
        .map(|(index, item)| item_line(item, index == view.selected, palette, chunks[0].width))
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines), chunks[0]);
    let status = view
        .status
        .clone()
        .unwrap_or_else(|| "↑/↓ navigate · Enter select · Esc close".to_string());
    frame.render_widget(
        Paragraph::new(Span::styled(
            status,
            Style::default().fg(color(palette, ColorToken::Muted)),
        )),
        chunks[1],
    );
}

fn palette_title(view: &PaletteView) -> &'static str {
    match view.level {
        PaletteLevel::Root => "Commands",
        PaletteLevel::Models => "Commands · Models",
        PaletteLevel::McpServers => "Commands · MCP servers",
        PaletteLevel::Skills => "Commands · Skills",
    }
}

fn item_line(
    item: &crate::entities::palette::item::PaletteItem,
    selected: bool,
    palette: &Palette,
    width: u16,
) -> Line<'static> {
    let mut spans = Vec::new();
    let marker = if selected { "› " } else { "  " };
    let label = format!("{marker}{}", item.label);
    let tone = tone_color(item.tone);
    let (foreground, background) = if !item.enabled {
        (color(palette, ColorToken::Muted), None)
    } else if selected {
        (
            color(palette, ColorToken::Background),
            Some(color(palette, ColorToken::Selection)),
        )
    } else {
        (color(palette, tone), None)
    };
    let mut style = Style::default().fg(foreground);
    if let Some(background) = background {
        style = style.bg(background);
    }
    if selected {
        style = style.add_modifier(Modifier::BOLD);
    }
    spans.push(Span::styled(label.clone(), style));
    if let Some(detail) = item.detail.as_ref() {
        let detail_text = format!("  {detail}");
        if item.detail_right {
            let used = label.chars().count() + detail_text.chars().count();
            let padding = (width as usize).saturating_sub(used);
            spans.push(Span::styled(" ".repeat(padding), style));
        }
        let detail_token = if item.tone == ItemTone::Default {
            ColorToken::Muted
        } else {
            tone
        };
        let mut detail_style = Style::default().fg(if item.enabled {
            color(palette, detail_token)
        } else {
            color(palette, ColorToken::Muted)
        });
        if let Some(background) = background {
            detail_style = detail_style.bg(background);
        }
        spans.push(Span::styled(detail_text, detail_style));
    }
    Line::from(spans)
}

fn tone_color(tone: ItemTone) -> ColorToken {
    match tone {
        ItemTone::Default => ColorToken::Text,
        ItemTone::Success => ColorToken::Success,
        ItemTone::Muted => ColorToken::Muted,
        ItemTone::Error => ColorToken::Error,
    }
}

fn color(palette: &Palette, token: ColorToken) -> Color {
    theme::token(palette, token)
}
