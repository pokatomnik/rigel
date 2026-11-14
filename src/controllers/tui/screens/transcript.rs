use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};

use crate::{
    controllers::tui::{json, theme},
    entities::{
        render_model::{
            model::RenderModel,
            transcript::{SubagentRow, ToolResultRow, ToolRow, TranscriptRow, TranscriptScroll},
        },
        theme::{palette::Palette, token::ColorToken},
        transcript::entry::EntryId,
    },
};

use super::Presentation;

/// Renders the transcript viewport with its exact wrapped-row scroll offset.
pub(crate) fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    presentation: &Presentation<'_>,
    scroll: &mut TranscriptScroll,
) {
    let palette = presentation.palette;
    let lines = transcript_lines(presentation.model, palette, area.width);
    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false }).style(
        Style::default()
            .fg(color(palette, ColorToken::Text))
            .bg(color(palette, ColorToken::Background)),
    );
    let total = paragraph.line_count(area.width);
    let offset = scroll.resolve(total, area.height as usize);
    frame.render_widget(paragraph.scroll((offset, 0)), area);
}

fn transcript_lines(model: &RenderModel, palette: &Palette, width: u16) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for row in &model.transcript.rows {
        render_row(&mut lines, model, palette, width, row);
    }
    lines
}

fn render_row(
    lines: &mut Vec<Line<'static>>,
    model: &RenderModel,
    palette: &Palette,
    width: u16,
    row: &TranscriptRow,
) {
    match row {
        TranscriptRow::User { text, .. } => render_user(lines, palette, text),
        TranscriptRow::Assistant {
            id,
            text,
            reasoning,
        } => render_assistant(lines, model, palette, *id, text, reasoning.as_deref()),
        TranscriptRow::Tool(tool) => render_tool(lines, model, palette, width, tool),
        TranscriptRow::LiveAssistant { text } => render_user(lines, palette, text),
        TranscriptRow::LiveReasoning { text, expanded } => {
            render_block(lines, palette, !expanded, "Thinking", text);
        }
        TranscriptRow::Summary { id, text } => {
            render_summary(lines, model, palette, *id, text);
        }
        TranscriptRow::Boundary { archive, .. } => {
            render_boundary(lines, palette, archive.as_str());
        }
    }
    lines.push(Line::default());
}

fn render_user(lines: &mut Vec<Line<'static>>, palette: &Palette, text: &str) {
    lines.push(label("You", color(palette, ColorToken::Accent)));
    push_text(lines, text, text_style(palette));
}

fn render_assistant(
    lines: &mut Vec<Line<'static>>,
    model: &RenderModel,
    palette: &Palette,
    id: EntryId,
    text: &str,
    reasoning: Option<&str>,
) {
    if !text.is_empty() {
        lines.push(label("Agent", color(palette, ColorToken::Success)));
    }
    if let Some(reasoning) = reasoning {
        let collapsed = !model.transcript.is_expanded(id);
        render_block(lines, palette, collapsed, "Thinking", reasoning);
    }
    if !text.is_empty() {
        push_text(lines, text, text_style(palette));
    }
}

fn render_summary(
    lines: &mut Vec<Line<'static>>,
    model: &RenderModel,
    palette: &Palette,
    id: EntryId,
    text: &str,
) {
    lines.push(label(
        "── History compacted; earlier messages archived ──",
        color(palette, ColorToken::Muted),
    ));
    render_block(
        lines,
        palette,
        !model.transcript.is_expanded(id),
        "Summary",
        text,
    );
}

fn render_boundary(lines: &mut Vec<Line<'static>>, palette: &Palette, archive: &str) {
    lines.push(label(
        format!("── Archive: {archive} ──").as_str(),
        color(palette, ColorToken::Muted),
    ));
}

fn render_block(
    lines: &mut Vec<Line<'static>>,
    palette: &Palette,
    collapsed: bool,
    title: &str,
    text: &str,
) {
    let marker = if collapsed { "[>]" } else { "[v]" };
    lines.push(label(
        format!("{marker} {title}").as_str(),
        color(palette, ColorToken::Muted),
    ));
    if !collapsed {
        push_text(lines, text, muted_style(palette));
    }
}

fn render_tool(
    lines: &mut Vec<Line<'static>>,
    model: &RenderModel,
    palette: &Palette,
    width: u16,
    tool: &ToolRow,
) {
    let expanded = model.transcript.is_expanded(tool.id);
    if let Some(subagent) = &tool.subagent {
        render_subagent(lines, palette, width, tool, subagent, expanded);
        return;
    }
    lines.push(tool_label(palette, expanded, tool.tool_name.as_str()));
    if expanded {
        render_tool_details(lines, palette, tool);
    }
}

fn render_tool_details(lines: &mut Vec<Line<'static>>, palette: &Palette, tool: &ToolRow) {
    push_json_or_text(lines, tool.arguments.as_str(), palette);
    if let Some(result) = &tool.result {
        render_tool_result(lines, palette, result);
    }
}

fn render_subagent(
    lines: &mut Vec<Line<'static>>,
    palette: &Palette,
    width: u16,
    tool: &ToolRow,
    subagent: &SubagentRow,
    expanded: bool,
) {
    lines.push(subagent_header(palette, width, tool, subagent, expanded));
    if expanded {
        render_subagent_details(lines, palette, subagent);
    }
}

fn subagent_header(
    palette: &Palette,
    width: u16,
    tool: &ToolRow,
    subagent: &SubagentRow,
    expanded: bool,
) -> Line<'static> {
    if expanded {
        return tool_label(palette, true, tool.tool_name.as_str());
    }
    let preview_width = (width as usize).saturating_sub(tool.tool_name.len() + 3);
    let preview = first_line_truncated(subagent.prompt.as_str(), preview_width);
    label(
        format!("[{}] {preview}", tool.tool_name).as_str(),
        color(palette, ColorToken::Warning),
    )
}

fn render_subagent_details(
    lines: &mut Vec<Line<'static>>,
    palette: &Palette,
    subagent: &SubagentRow,
) {
    lines.push(label("Task", color(palette, ColorToken::Muted)));
    push_text(lines, subagent.prompt.as_str(), text_style(palette));
    if !subagent.activity.trim().is_empty() {
        lines.push(label("Work", color(palette, ColorToken::Muted)));
        push_text(lines, subagent.activity.as_str(), muted_style(palette));
    }
    if let Some(report) = subagent.report.as_deref() {
        lines.push(label("Report", color(palette, ColorToken::Success)));
        push_text(lines, report, text_style(palette));
    }
}

fn render_tool_result(lines: &mut Vec<Line<'static>>, palette: &Palette, result: &ToolResultRow) {
    if result.failed {
        let style = Style::default().fg(color(palette, ColorToken::Error));
        push_text(lines, result.output.as_str(), style);
        return;
    }
    push_json_or_text(lines, result.output.as_str(), palette);
}

fn push_json_or_text(lines: &mut Vec<Line<'static>>, text: &str, palette: &Palette) {
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(value) => lines.extend(json::lines(&value, palette)),
        Err(_) => push_text(lines, text, text_style(palette)),
    }
}

fn tool_label(palette: &Palette, expanded: bool, text: &str) -> Line<'static> {
    let text = if expanded {
        format!("[v] {text}")
    } else {
        format!("[{text}]")
    };
    label(text.as_str(), color(palette, ColorToken::Warning))
}

fn first_line_truncated(text: &str, width: usize) -> String {
    let first = text.lines().next().unwrap_or("").trim();
    if first.chars().count() <= width {
        return first.to_string();
    }
    let mut truncated = first
        .chars()
        .take(width.saturating_sub(1))
        .collect::<String>();
    truncated.push('…');
    truncated
}

fn label(text: &str, color: Color) -> Line<'static> {
    Line::from(Span::styled(
        text.to_string(),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    ))
}

fn text_style(palette: &Palette) -> Style {
    Style::default().fg(color(palette, ColorToken::Text))
}

fn muted_style(palette: &Palette) -> Style {
    Style::default().fg(color(palette, ColorToken::Muted))
}

fn color(palette: &Palette, token: ColorToken) -> Color {
    theme::token(palette, token)
}

fn push_text(lines: &mut Vec<Line<'static>>, text: &str, style: Style) {
    if text.is_empty() {
        lines.push(Line::default());
        return;
    }
    for line in text.lines() {
        lines.push(Line::from(Span::styled(line.to_string(), style)));
    }
}
