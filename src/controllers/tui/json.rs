use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

use crate::{
    controllers::tui::theme,
    entities::theme::{palette::Palette, token::ColorToken},
};

/// Renders a JSON value as pretty-printed, syntax-highlighted lines.
pub(crate) fn lines(value: &serde_json::Value, palette: &Palette) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    push_value(&mut lines, value, 0, palette, None);
    if lines.is_empty() {
        lines.push(Line::default());
    }
    lines
}

fn push_value(
    lines: &mut Vec<Line<'static>>,
    value: &serde_json::Value,
    indent: usize,
    palette: &Palette,
    prefix: Option<&str>,
) {
    match value {
        serde_json::Value::Object(object) => {
            push_open(lines, "{", indent, prefix, palette);
            let last = object.len().saturating_sub(1);
            for (index, (key, value)) in object.iter().enumerate() {
                let key = format!("\"{key}\": ");
                let comma = (index < last).then_some(",");
                let mut head = Vec::new();
                head.push(Span::styled(" ".repeat((indent + 1) * 2), Style::default()));
                head.push(Span::styled(
                    key,
                    Style::default()
                        .fg(color(palette, ColorToken::Accent))
                        .add_modifier(Modifier::BOLD),
                ));
                match value {
                    serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
                        lines.push(Line::from(head));
                        push_value(lines, value, indent + 1, palette, comma);
                    }
                    _ => {
                        let mut spans = head;
                        push_scalar(&mut spans, value, palette);
                        if let Some(comma) = comma {
                            spans.push(Span::styled(comma, punctuation(palette)));
                        }
                        lines.push(Line::from(spans));
                    }
                }
            }
            lines.push(Line::from(Span::styled(
                format!("{}{}", " ".repeat(indent * 2), "}"),
                punctuation(palette),
            )));
        }
        serde_json::Value::Array(items) => {
            push_open(lines, "[", indent, prefix, palette);
            let last = items.len().saturating_sub(1);
            for (index, item) in items.iter().enumerate() {
                let comma = (index < last).then_some(",");
                match item {
                    serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
                        push_value(lines, item, indent + 1, palette, comma);
                    }
                    _ => {
                        let mut spans =
                            vec![Span::styled(" ".repeat((indent + 1) * 2), Style::default())];
                        push_scalar(&mut spans, item, palette);
                        if let Some(comma) = comma {
                            spans.push(Span::styled(comma, punctuation(palette)));
                        }
                        lines.push(Line::from(spans));
                    }
                }
            }
            lines.push(Line::from(Span::styled(
                format!("{}{}", " ".repeat(indent * 2), "]"),
                punctuation(palette),
            )));
        }
        _ => {
            let mut spans = Vec::new();
            if let Some(prefix) = prefix {
                spans.push(Span::styled(prefix.to_string(), punctuation(palette)));
            }
            push_scalar(&mut spans, value, palette);
            lines.push(Line::from(spans));
        }
    }
}

fn push_open(
    lines: &mut Vec<Line<'static>>,
    bracket: &str,
    indent: usize,
    prefix: Option<&str>,
    palette: &Palette,
) {
    let mut spans = vec![Span::styled(
        format!("{}{}", " ".repeat(indent * 2), bracket),
        punctuation(palette),
    )];
    if let Some(prefix) = prefix {
        spans.push(Span::styled(prefix.to_string(), punctuation(palette)));
    }
    lines.push(Line::from(spans));
}

fn push_scalar(spans: &mut Vec<Span<'static>>, value: &serde_json::Value, palette: &Palette) {
    let (text, token) = match value {
        serde_json::Value::String(text) => (format!("\"{text}\""), ColorToken::Success),
        serde_json::Value::Number(number) => (number.to_string(), ColorToken::Warning),
        serde_json::Value::Bool(boolean) => (boolean.to_string(), ColorToken::Warning),
        serde_json::Value::Null => ("null".to_string(), ColorToken::Muted),
        serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
            (String::new(), ColorToken::Text)
        }
    };
    spans.push(Span::styled(
        text,
        Style::default().fg(color(palette, token)),
    ));
}

fn punctuation(palette: &Palette) -> Style {
    Style::default().fg(color(palette, ColorToken::Muted))
}

fn color(palette: &Palette, token: ColorToken) -> ratatui::style::Color {
    theme::token(palette, token)
}

#[cfg(test)]
mod tests {
    use super::lines;
    use crate::entities::theme::palette::Palette;

    #[test]
    fn object_keys_and_string_values_keep_their_payloads() {
        let value = serde_json::json!({"path": "src/main.rs", "count": 2});
        let lines = lines(&value, &Palette::rigel_night());
        let text = lines
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(text.contains("\"path\""));
        assert!(text.contains("\"src/main.rs\""));
        assert!(text.contains("count"));
        assert!(text.contains('2'));
    }

    #[test]
    fn nested_arrays_and_objects_are_indented() {
        let value = serde_json::json!({"items": [{"ok": true}]});
        let text = lines(&value, &Palette::rigel_night())
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(text.contains("  \"items\""));
        assert!(text.contains("    {"));
        assert!(text.contains("      \"ok\""));
    }
}
