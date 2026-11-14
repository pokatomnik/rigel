use rig::message::{Text, ToolCall, ToolResult, ToolResultContent};

use crate::{
    controllers::model_runtime::recovery::tool_recovery::tool_recovery_status,
    controllers::terminal::terminal_io::TerminalIO, shared::string::string_ext::StringShort,
};

use super::stream_output_state::StreamOutputState;

pub(crate) fn print_reasoning(
    terminal_io: &TerminalIO,
    state: &mut StreamOutputState,
    reasoning: &str,
) {
    if reasoning.is_empty() {
        return;
    }
    if !reasoning.trim().is_empty() {
        state.set_received_reasoning(true);
        if !state.showing_reasoning() {
            terminal_io.eprintln_gray("[thinking]");
            state.set_showing_reasoning(true);
        }
    }
    terminal_io.eprint_gray(reasoning);
    terminal_io.flush_stderr();
}

/// Routes one streamed turn's display to either the terminal or the live event sink.
pub(crate) struct StreamDisplay<'a> {
    terminal_io: &'a TerminalIO,
    state: &'a mut StreamOutputState,
}

impl<'a> StreamDisplay<'a> {
    /// Creates a display target for one streamed turn.
    pub(crate) fn new(terminal_io: &'a TerminalIO, state: &'a mut StreamOutputState) -> Self {
        Self { terminal_io, state }
    }

    /// Streams reasoning only when no live event sink is active.
    pub(crate) fn reasoning(&mut self, reasoning: &str, live: bool) {
        if live {
            if !reasoning.trim().is_empty() {
                self.state.set_received_reasoning(true);
            }
            return;
        }
        print_reasoning(self.terminal_io, self.state, reasoning);
    }

    /// Streams answer text only when no live event sink is active.
    pub(crate) fn text(&mut self, text: &Text, live: bool) {
        if live {
            if !text.text().trim().is_empty() {
                self.state.set_received_answer(true);
            }
            return;
        }
        print_text(self.terminal_io, self.state, text);
    }

    /// Records a tool result for recovery and prints it only without live events.
    pub(crate) fn tool_result(&mut self, tool_result: &ToolResult, live: bool) {
        if live {
            self.state
                .record_tool_result(tool_recovery_status(tool_result));
            return;
        }
        print_tool_result(self.terminal_io, self.state, tool_result);
    }
}

/// Prints a tool call only when no live event sink is active.
pub(crate) fn stream_tool_call(terminal_io: &TerminalIO, tool_call: &ToolCall, live: bool) {
    if !live {
        print_tool_call(terminal_io, tool_call);
    }
}

pub(crate) fn print_text(terminal_io: &TerminalIO, state: &mut StreamOutputState, text: &Text) {
    if text.text().is_empty() {
        return;
    }
    if !text.text().trim().is_empty() {
        state.set_received_answer(true);
        if state.showing_reasoning() {
            terminal_io.eprintln("\n[answer]");
            state.set_showing_reasoning(false);
        }
    }
    terminal_io.print(text.text());
    terminal_io.flush_stdout();
}

pub(crate) fn print_tool_call(terminal_io: &TerminalIO, tool_call: &ToolCall) {
    terminal_io.eprintln_orange(format_tool_call(tool_call).as_str());
}

pub(crate) fn print_tool_result(
    terminal_io: &TerminalIO,
    state: &mut StreamOutputState,
    tool_result: &ToolResult,
) {
    state.record_tool_result(tool_recovery_status(tool_result));
    terminal_io.eprintln_blue(format_tool_result(tool_result).as_str());
}

fn format_tool_call(tool_call: &ToolCall) -> String {
    format!(
        "\n[tool call: {}({})]",
        tool_call.function.name, tool_call.function.arguments
    )
}

fn format_tool_result(tool_result: &ToolResult) -> String {
    let output = tool_result
        .content
        .iter()
        .map(format_tool_result_content)
        .collect::<Vec<_>>()
        .join("\n");
    format!("[tool result: {}]", output.short(30))
}

/// Formats a complete tool result for the expanded transcript block without truncation.
pub(crate) fn format_tool_result_full(tool_result: &ToolResult) -> String {
    tool_result
        .content
        .iter()
        .filter(|content| !is_recovery_marker(content))
        .map(format_pretty_content)
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_pretty_content(content: &ToolResultContent) -> String {
    match content {
        ToolResultContent::Text(text) => text.text.clone(),
        ToolResultContent::Json { value } => {
            serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
        }
        ToolResultContent::Image(image) => format_image(image),
    }
}

fn format_image(image: &rig::message::Image) -> String {
    use rig::message::DocumentSourceKind;
    let source = match &image.data {
        DocumentSourceKind::Url(url) => format!("url={url}"),
        DocumentSourceKind::Base64(data) => format!("base64, {} bytes", data.len()),
        DocumentSourceKind::FileId(id) => format!("file_id={id}"),
        DocumentSourceKind::Raw(data) => format!("raw, {} bytes", data.len()),
        DocumentSourceKind::String(_) => "source=unknown".to_string(),
        _ => "source=unknown".to_string(),
    };
    let media_type = image
        .media_type
        .as_ref()
        .map(|media| format!("{media:?}"))
        .unwrap_or_else(|| "unknown".to_string());
    format!("<image {media_type}; {source}>")
}

fn is_recovery_marker(content: &ToolResultContent) -> bool {
    content
        .as_json()
        .is_some_and(|value| value.get("rigel_tool_status").is_some())
}

fn format_tool_result_content(content: &ToolResultContent) -> String {
    match content {
        ToolResultContent::Text(text) => text.text.clone(),
        ToolResultContent::Json { value } => value.to_string(),
        ToolResultContent::Image(_) => "<image>".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use rig::{OneOrMany, message::ToolFunction};

    use super::{format_tool_call, format_tool_result};
    use rig::message::{ToolCall, ToolResult, ToolResultContent};

    fn tool_result(content: OneOrMany<ToolResultContent>) -> ToolResult {
        ToolResult {
            id: "call-1".to_string(),
            call_id: None,
            content,
        }
    }

    #[test]
    fn tool_call_arguments_are_displayed_in_full() {
        let arguments = serde_json::json!({
            "query": "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"
        });
        let call = ToolCall::new(
            "call-1".to_string(),
            ToolFunction::new("search".to_string(), arguments.clone()),
        );

        let formatted = format_tool_call(&call);

        assert!(formatted.contains(arguments.to_string().as_str()));
        assert!(!formatted.contains("..."));
    }

    #[test]
    fn short_tool_call_arguments_are_unchanged() {
        let call = ToolCall::new(
            "call-1".to_string(),
            ToolFunction::new("search".to_string(), serde_json::json!({"query": "rust"})),
        );

        assert_eq!(
            format_tool_call(&call),
            "\n[tool call: search({\"query\":\"rust\"})]"
        );
    }

    #[test]
    fn long_tool_result_keeps_thirty_unicode_characters_on_each_side() {
        let content = format!("{}middle{}", "你".repeat(30), "界".repeat(30));

        let formatted = format_tool_result(&tool_result(OneOrMany::one(ToolResultContent::text(
            content,
        ))));

        assert_eq!(
            formatted,
            format!("[tool result: {}...{}]", "你".repeat(30), "界".repeat(30))
        );
    }

    #[test]
    fn short_tool_result_is_unchanged() {
        let result = tool_result(OneOrMany::one(ToolResultContent::text("done")));

        assert_eq!(format_tool_result(&result), "[tool result: done]");
    }

    #[test]
    fn multi_part_tool_result_is_shortened_after_joining() {
        let mut content = OneOrMany::one(ToolResultContent::text("a".repeat(40)));
        content.push(ToolResultContent::text("b".repeat(40)));

        let formatted = format_tool_result(&tool_result(content));

        assert_eq!(
            formatted,
            format!("[tool result: {}...{}]", "a".repeat(30), "b".repeat(30))
        );
    }

    #[test]
    fn json_and_image_tool_result_content_keep_their_display_forms() {
        let mut content = OneOrMany::one(ToolResultContent::json(serde_json::json!({
            "ok": true
        })));
        content.push(ToolResultContent::image_base64("encoded", None, None));

        assert_eq!(
            format_tool_result(&tool_result(content)),
            "[tool result: {\"ok\":true}\n<image>]"
        );
    }
}
