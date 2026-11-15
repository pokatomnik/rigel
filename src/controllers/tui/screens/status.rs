use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::{
    controllers::tui::theme,
    entities::{
        agent_event::status::{AgentActivity, NoticeLevel},
        application::input_event::SUBMIT_KEY_LABEL,
        theme::{palette::Palette, token::ColorToken},
    },
};

use super::Presentation;

fn idle_hint() -> String {
    format!("{SUBMIT_KEY_LABEL}: send · Alt+Enter: newline · Ctrl+K")
}

/// Renders the single-line status, notice, or read-only banner.
pub(crate) fn render(frame: &mut Frame<'_>, area: Rect, presentation: &Presentation<'_>) {
    let palette = presentation.palette;
    let status = &presentation.model.status;
    let (text, token) = if status.read_only.active {
        let reason = status
            .read_only
            .reason
            .as_deref()
            .unwrap_or("history writes are blocked");
        (
            format!("Read-only: {reason}. Use Retry in the palette."),
            ColorToken::Error,
        )
    } else if let Some(notice) = status.notices.last() {
        (notice.text.clone(), notice_color(notice.level))
    } else {
        activity_text(status.activity, status.hint.as_deref())
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            text,
            Style::default().fg(color(palette, token)),
        )))
        .style(Style::default().bg(color(palette, ColorToken::Background))),
        area,
    );
}

fn color(palette: &Palette, token: ColorToken) -> Color {
    theme::token(palette, token)
}

fn activity_text(activity: AgentActivity, hint: Option<&str>) -> (String, ColorToken) {
    match activity {
        AgentActivity::Idle => (
            hint.map(ToOwned::to_owned).unwrap_or_else(idle_hint),
            ColorToken::Muted,
        ),
        AgentActivity::Running => ("Working... Esc to stop".to_string(), ColorToken::Accent),
        AgentActivity::Stopping => ("Stopping...".to_string(), ColorToken::Warning),
        AgentActivity::Compacting => ("Compacting...".to_string(), ColorToken::Warning),
    }
}

fn notice_color(level: NoticeLevel) -> ColorToken {
    match level {
        NoticeLevel::Info => ColorToken::Accent,
        NoticeLevel::Warning => ColorToken::Warning,
        NoticeLevel::Error => ColorToken::Error,
        NoticeLevel::Success => ColorToken::Success,
    }
}

#[cfg(test)]
mod tests {
    use super::{activity_text, notice_color};
    use crate::entities::{
        agent_event::status::{AgentActivity, NoticeLevel},
        theme::token::ColorToken,
    };

    #[test]
    fn stopping_and_compacting_are_visible_states() {
        assert_eq!(
            activity_text(AgentActivity::Stopping, None),
            ("Stopping...".to_string(), ColorToken::Warning)
        );
        assert_eq!(
            activity_text(AgentActivity::Compacting, None),
            ("Compacting...".to_string(), ColorToken::Warning)
        );
    }

    #[test]
    fn every_notice_level_has_a_semantic_color() {
        assert_eq!(notice_color(NoticeLevel::Info), ColorToken::Accent);
        assert_eq!(notice_color(NoticeLevel::Warning), ColorToken::Warning);
        assert_eq!(notice_color(NoticeLevel::Error), ColorToken::Error);
        assert_eq!(notice_color(NoticeLevel::Success), ColorToken::Success);
    }
}
