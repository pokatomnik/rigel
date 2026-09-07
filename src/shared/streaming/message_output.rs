use rig::{
    OneOrMany,
    message::{
        AssistantContent, Message, Reasoning, Text, ToolCall, ToolResult, ToolResultContent,
        UserContent,
    },
};

use crate::{
    entities::context_usage::ContextUsage,
    shared::recovery::tool_recovery::tool_recovery_status,
    shared::{string::string_ext::StringShort, terminal::terminal_io::TerminalIO},
};

use super::stream_output_state::StreamOutputState;

pub(crate) fn display_message(
    terminal_io: &TerminalIO,
    state: &mut StreamOutputState,
    message: &Message,
) -> bool {
    match message {
        Message::System { .. } => false,
        Message::User { content } => {
            for content in content.iter() {
                display_user_content(terminal_io, state, content);
            }
            false
        }
        Message::Assistant { content, .. } => {
            for content in content.iter() {
                display_assistant_content(terminal_io, state, content);
            }
            assistant_content_needs_line_break(content)
        }
    }
}

fn display_user_content(
    terminal_io: &TerminalIO,
    state: &mut StreamOutputState,
    content: &UserContent,
) {
    match content {
        UserContent::Text(text) => {
            state.reset();
            print_user_text(terminal_io, text);
        }
        UserContent::ToolResult(tool_result) => print_tool_result(terminal_io, state, tool_result),
        UserContent::Image(_)
        | UserContent::Audio(_)
        | UserContent::Video(_)
        | UserContent::Document(_) => {}
    }
}

fn display_assistant_content(
    terminal_io: &TerminalIO,
    state: &mut StreamOutputState,
    content: &AssistantContent,
) {
    match content {
        AssistantContent::Reasoning(reasoning) => {
            print_reasoning_block(terminal_io, state, reasoning)
        }
        AssistantContent::Text(text) => print_text(terminal_io, state, text),
        AssistantContent::ToolCall(tool_call) => print_tool_call(terminal_io, tool_call),
        AssistantContent::Image(_) => {}
    }
}

fn print_user_text(terminal_io: &TerminalIO, text: &Text) {
    terminal_io.print_prompt_prefix(ContextUsage::new(None, None));
    terminal_io.print(text.text());
    terminal_io.print("\n");
}

fn assistant_content_needs_line_break(content: &OneOrMany<AssistantContent>) -> bool {
    content
        .iter()
        .fold(false, |needs_line_break, content| match content {
            AssistantContent::Reasoning(reasoning) if !reasoning.display_text().is_empty() => true,
            AssistantContent::Text(text) if !text.text().is_empty() => true,
            AssistantContent::ToolCall(_) => false,
            _ => needs_line_break,
        })
}

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

pub(crate) fn print_reasoning_block(
    terminal_io: &TerminalIO,
    state: &mut StreamOutputState,
    reasoning: &Reasoning,
) {
    print_reasoning(terminal_io, state, reasoning.display_text().as_str());
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
