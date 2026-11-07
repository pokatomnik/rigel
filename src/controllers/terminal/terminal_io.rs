use std::{
    fmt::Display,
    io::{self, Write},
};

use crate::{
    controllers::model_runtime::{
        history::message_codec::to_provider,
        streaming::{message_output::display_message, stream_output_state::StreamOutputState},
    },
    entities::{
        conversation::{context_usage::ContextUsage, conversation_message::ConversationMessage},
        skill::definition::SkillDescriptor,
        tool_confirm_result::outcome::ToolConfirmResult,
    },
    infrastructure::{application::ApplicationOutput, interaction::ChatIO},
};

/// Describes the non-error outcomes of one line read from the terminal.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ReadlineOutcome {
    /// A line was read, with the existing prompt whitespace removed.
    Text(String),
    /// The input stream reached EOF before a line was read.
    Eof,
    /// The user interrupted the line read.
    Cancelled,
}

#[derive(Default)]
pub(crate) struct TerminalIO;

impl ApplicationOutput for TerminalIO {
    fn eprintln(&self, message: &str) {
        TerminalIO::eprintln(self, message);
    }
}

impl ChatIO for TerminalIO {
    fn readline(
        &self,
        context_usage: ContextUsage,
    ) -> anyhow::Result<crate::entities::application::input_outcome::InputOutcome> {
        Ok(match TerminalIO::readline(self, context_usage)? {
            ReadlineOutcome::Text(text) => {
                crate::entities::application::input_outcome::InputOutcome::Text(text)
            }
            ReadlineOutcome::Eof => crate::entities::application::input_outcome::InputOutcome::Eof,
            ReadlineOutcome::Cancelled => {
                crate::entities::application::input_outcome::InputOutcome::Cancelled
            }
        })
    }

    fn editor(&self) -> anyhow::Result<String> {
        TerminalIO::editor(self)
    }

    fn print(&self, message: &str) {
        TerminalIO::print(self, message);
    }

    fn eprintln(&self, message: &str) {
        TerminalIO::eprintln(self, message);
    }

    fn eprintln_gray(&self, message: &str) {
        TerminalIO::eprintln_gray(self, message);
    }

    fn eprintln_red(&self, message: &str) {
        TerminalIO::eprintln_red(self, message);
    }

    fn eprintln_orange(&self, message: &str) {
        TerminalIO::eprintln_orange(self, message);
    }

    fn select_skill(&self, items: &[SkillDescriptor]) -> anyhow::Result<Option<usize>> {
        if items.is_empty() {
            return Ok(None);
        }
        dialoguer::FuzzySelect::new()
            .items(items)
            .with_prompt("Select skill")
            .clear(true)
            .default(0)
            .report(true)
            .interact()
            .map(Some)
            .map_err(Into::into)
    }

    fn display_history(&self, messages: &[ConversationMessage]) {
        let Ok(provider_messages) = to_provider(messages) else {
            return;
        };
        let mut state = StreamOutputState::default();
        for message in &provider_messages {
            if display_message(self, &mut state, message) {
                self.eprintln("");
            }
        }
    }
}

impl TerminalIO {
    pub fn print(&self, msg: &str) {
        print!("{msg}")
    }

    pub fn eprintln(&self, msg: &str) {
        eprintln!("{msg}");
    }

    pub fn eprintln_gray(&self, msg: &str) {
        let style = console::Style::new().black().bright().for_stderr();
        eprintln!("{}", style.apply_to(msg));
    }

    pub fn eprintln_red(&self, msg: &str) {
        let style = console::Style::new().red().for_stderr();
        eprintln!("{}", style.apply_to(msg));
    }

    pub fn eprint_gray(&self, msg: &str) {
        let style = console::Style::new().black().bright().for_stderr();
        eprint!("{}", style.apply_to(msg));
    }

    pub fn eprintln_orange(&self, msg: &str) {
        let style = console::Style::new().yellow().bright().for_stderr();
        eprintln!("{}", style.apply_to(msg));
    }

    pub fn eprintln_blue(&self, msg: &str) {
        let style = console::Style::new().cyan().for_stderr();
        eprintln!("{}", style.apply_to(msg));
    }

    pub fn flush_stderr(&self) {
        let _ = io::stderr().flush();
    }

    pub fn flush_stdout(&self) {
        let _ = io::stdout().flush();
    }

    pub fn print_prompt_prefix(&self, context_usage: ContextUsage) {
        self.eprint_gray(format_context_prompt(context_usage).as_str());
    }

    pub fn confirm_tool_call(&self, prompt: &str) -> ToolConfirmResult {
        let items: &'static [ToolConfirmResult] = &[
            ToolConfirmResult::Deny,
            ToolConfirmResult::AllowOnce,
            ToolConfirmResult::AllowForProject,
            ToolConfirmResult::AllowForSession,
            ToolConfirmResult::AllowForever,
        ];
        let answer_idx = dialoguer::Select::new()
            .with_prompt(prompt)
            .items(items)
            .default(0)
            .interact()
            .unwrap_or_default();
        items
            .get(answer_idx)
            .map(ToOwned::to_owned)
            .unwrap_or_default()
    }

    pub fn fuzzy_select<'a, I>(&self, prompt: &str, items: &'a [I]) -> anyhow::Result<&'a I>
    where
        I: Display,
    {
        if items.is_empty() {
            anyhow::bail!("Empty items list");
        }

        let idx = dialoguer::FuzzySelect::new()
            .items(items)
            .with_prompt(prompt)
            .clear(true)
            .default(0)
            .report(true)
            .interact()?;
        if let Some(item) = items.get(idx) {
            return Ok(item);
        }

        anyhow::bail!("No item selected")
    }

    pub fn readline(&self, context_usage: ContextUsage) -> anyhow::Result<ReadlineOutcome> {
        self.print_prompt_prefix(context_usage);
        let mut line = String::new();
        match io::stdin().read_line(&mut line) {
            Ok(0) => Ok(ReadlineOutcome::Eof),
            Ok(_) => Ok(ReadlineOutcome::Text(line.trim().to_string())),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                Ok(ReadlineOutcome::Cancelled)
            }
            Err(error) => Err(error.into()),
        }
    }

    pub fn editor(&self) -> anyhow::Result<String> {
        let mut result = String::new();
        while result.trim().is_empty() {
            result = dialoguer::Editor::new()
                .require_save(false)
                .edit("")?
                .unwrap_or_default();
        }

        Ok(result)
    }
}

fn format_context_prompt(context_usage: ContextUsage) -> String {
    let (used, max) = context_usage.values();
    match (used, max) {
        (Some(used), Some(max)) => format!(
            "{}/{} > ",
            format_token_count(used),
            format_token_count(max)
        ),
        (Some(used), None) => format!("{} > ", format_token_count(used)),
        (None, Some(max)) => format!("?/{} > ", format_token_count(max)),
        (None, None) => "> ".to_string(),
    }
}

fn format_token_count(value: u64) -> String {
    if value < 1_000 {
        return value.to_string();
    }
    if value < 1_000_000 {
        return format!("{}K", value / 1_000);
    }
    if value < 1_000_000_000 {
        return format!("{}M", value / 1_000_000);
    }
    if value < 1_000_000_000_000 {
        return format!("{}B", value / 1_000_000_000);
    }
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::{format_context_prompt, format_token_count};
    use crate::entities::conversation::context_usage::ContextUsage;

    #[test]
    fn formats_token_count_at_each_boundary() {
        for (value, expected) in [
            (999, "999"),
            (1_000, "1K"),
            (999_999, "999K"),
            (1_000_000, "1M"),
            (999_999_999, "999M"),
            (1_000_000_000, "1B"),
            (999_999_999_999, "999B"),
            (1_000_000_000_000, "1000000000000"),
        ] {
            assert_eq!(format_token_count(value), expected);
        }
    }

    #[test]
    fn formats_prompt_for_each_usage_combination() {
        assert_eq!(
            format_context_prompt(ContextUsage::new(Some(123), Some(4_000))),
            "123/4K > "
        );
        assert_eq!(
            format_context_prompt(ContextUsage::new(Some(123_000), None)),
            "123K > "
        );
        assert_eq!(
            format_context_prompt(ContextUsage::new(None, Some(1_000_000))),
            "?/1M > "
        );
        assert_eq!(format_context_prompt(ContextUsage::new(None, None)), "> ");
    }
}
