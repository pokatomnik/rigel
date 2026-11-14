use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::{
    controllers::tui::theme,
    entities::{
        agent_event::status::AgentActivity,
        conversation::context_usage::ContextUsage,
        render_model::views::{McpSummary, format_token_count},
        theme::{palette::Palette, token::ColorToken},
    },
};

use super::Presentation;

/// Renders the single-line header with model, MCP, context, and activity.
pub(crate) fn render(frame: &mut Frame<'_>, area: Rect, presentation: &Presentation<'_>) {
    let palette = presentation.palette;
    let header = &presentation.model.header;
    let mut spans = vec![Span::styled(
        "Rigel",
        Style::default()
            .fg(color(palette, ColorToken::Text))
            .add_modifier(Modifier::BOLD),
    )];
    if let Some(model) = &header.model {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            model.clone(),
            Style::default().fg(color(palette, ColorToken::Accent)),
        ));
    }
    if header.mcp.total > 0 {
        spans.push(Span::raw("   "));
        spans.push(Span::styled(
            format!("MCP {}/{}", header.mcp.running, header.mcp.total),
            Style::default().fg(color(palette, mcp_color(header.mcp))),
        ));
    }
    spans.push(Span::raw("   "));
    spans.push(Span::styled(
        format_context(header.context),
        Style::default().fg(color(palette, ColorToken::Muted)),
    ));
    let (label, token) = activity_label(header.activity);
    spans.push(Span::raw("   "));
    spans.push(Span::styled(
        label,
        Style::default().fg(color(palette, token)),
    ));
    frame.render_widget(
        Paragraph::new(Line::from(spans))
            .style(Style::default().bg(color(palette, ColorToken::Background))),
        area,
    );
}

fn color(palette: &Palette, token: ColorToken) -> Color {
    theme::token(palette, token)
}

fn mcp_color(summary: McpSummary) -> ColorToken {
    if summary.running == summary.total {
        ColorToken::Success
    } else if summary.starting > 0 {
        ColorToken::Muted
    } else {
        ColorToken::Error
    }
}

fn activity_label(activity: AgentActivity) -> (&'static str, ColorToken) {
    match activity {
        AgentActivity::Idle => ("idle", ColorToken::Muted),
        AgentActivity::Running => ("working", ColorToken::Accent),
        AgentActivity::Stopping => ("stopping", ColorToken::Warning),
        AgentActivity::Compacting => ("compacting", ColorToken::Warning),
    }
}

fn format_context(usage: ContextUsage) -> String {
    match usage.values() {
        (Some(used), Some(max)) => {
            format!(
                "context {}/{}",
                format_token_count(used),
                format_token_count(max)
            )
        }
        (Some(used), None) => format!("context {}", format_token_count(used)),
        (None, Some(max)) => format!("context ?/{}", format_token_count(max)),
        (None, None) => "context unknown".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{format_context, mcp_color};
    use crate::entities::{
        conversation::context_usage::ContextUsage, render_model::views::format_token_count,
        theme::token::ColorToken,
    };

    #[test]
    fn token_counts_are_abbreviated_at_each_boundary() {
        for (value, expected) in [
            (999, "999"),
            (1_000, "1K"),
            (999_999, "999K"),
            (1_000_000, "1M"),
            (1_000_000_000, "1B"),
            (1_000_000_000_000, "1000000000000"),
        ] {
            assert_eq!(format_token_count(value), expected);
        }
    }

    #[test]
    fn context_formats_every_known_combination() {
        assert_eq!(
            format_context(ContextUsage::new(Some(123), Some(4_000))),
            "context 123/4K"
        );
        assert_eq!(
            format_context(ContextUsage::new(Some(123_000), None)),
            "context 123K"
        );
        assert_eq!(
            format_context(ContextUsage::new(None, Some(1_000_000))),
            "context ?/1M"
        );
        assert_eq!(
            format_context(ContextUsage::new(None, None)),
            "context unknown"
        );
    }

    #[test]
    fn mcp_color_reports_full_partial_and_pending_connections() {
        use crate::entities::render_model::views::McpSummary;

        assert_eq!(mcp_color(McpSummary::new(3, 3)), ColorToken::Success);
        assert_eq!(mcp_color(McpSummary::new(2, 3)), ColorToken::Error);
        assert_eq!(
            mcp_color(McpSummary::new(2, 3).with_starting(1)),
            ColorToken::Muted
        );
    }
}
