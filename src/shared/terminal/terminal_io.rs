use std::{
    fmt::Display,
    io::{self, Write},
};

use crate::entities::tool_confirm_result::ToolConfirmResult;

#[derive(Default)]
pub(crate) struct TerminalIO;

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

    pub fn confirm_tool_call(&self, prompt: &str) -> ToolConfirmResult {
        let items: &'static [ToolConfirmResult] =
            &[ToolConfirmResult::No, ToolConfirmResult::AllowOnce];
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

    pub fn readline(&self) -> anyhow::Result<String> {
        self.eprint_gray("> ");
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        Ok(line.trim().to_string())
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
