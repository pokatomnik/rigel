use std::{
    fmt::Display,
    io::{self, Write},
};

use dialoguer::theme::ColorfulTheme;

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

    pub fn flush_stderr(&self) {
        let _ = io::stderr().flush();
    }

    pub fn flush_stdout(&self) {
        let _ = io::stdout().flush();
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
        let result =
            dialoguer::Input::<String>::with_theme(&ColorfulTheme::default()).interact_text()?;
        Ok(result)
    }
}
