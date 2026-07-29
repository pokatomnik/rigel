use std::{
    fmt::Display,
    io::{self, Write},
};

pub(crate) enum ReadlineResult {
    Line(String),
    Cancel,
}

pub(crate) struct TerminalIO {
    prompt: String,
}

impl TerminalIO {
    pub fn new(prompt: impl Into<String>) -> anyhow::Result<Self> {
        Ok(Self {
            prompt: prompt.into(),
        })
    }

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

    pub fn readline(&self) -> anyhow::Result<ReadlineResult> {
        self.print(self.prompt.as_str());
        self.flush_stdout();

        let mut line = String::new();
        if io::stdin().read_line(&mut line)? == 0 {
            return Ok(ReadlineResult::Cancel);
        }

        let line = line.trim_end_matches(['\r', '\n']).to_owned();
        Ok(ReadlineResult::Line(line))
    }
}
