use std::io::{self, IsTerminal, Stdout};

use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    crossterm::{
        cursor::Show,
        event::{DisableBracketedPaste, EnableBracketedPaste},
        execute,
        terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
    },
};

/// Owns terminal modes and restores them on every normal and error exit path.
pub(crate) struct TuiLifecycle {
    terminal: Option<Terminal<CrosstermBackend<Stdout>>>,
    active: bool,
}

impl TuiLifecycle {
    /// Rejects a non-interactive invocation before any terminal mode is changed.
    pub(crate) fn ensure_interactive() -> anyhow::Result<()> {
        if io::stdin().is_terminal() && io::stdout().is_terminal() {
            return Ok(());
        }
        anyhow::bail!("`rigel chat` requires an interactive terminal");
    }

    /// Enters raw mode and the alternate screen.
    pub(crate) fn enter() -> anyhow::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        if let Err(error) = execute!(stdout, EnterAlternateScreen, EnableBracketedPaste) {
            let _ = disable_raw_mode();
            return Err(error.into());
        }
        match Terminal::new(CrosstermBackend::new(stdout)) {
            Ok(terminal) => Ok(Self {
                terminal: Some(terminal),
                active: true,
            }),
            Err(error) => {
                let _ = disable_raw_mode();
                let _ = execute!(io::stdout(), DisableBracketedPaste, LeaveAlternateScreen);
                Err(error.into())
            }
        }
    }

    /// Transfers the terminal to the renderer while this guard keeps restoration.
    pub(crate) fn take_terminal(&mut self) -> anyhow::Result<Terminal<CrosstermBackend<Stdout>>> {
        self.terminal
            .take()
            .ok_or_else(|| anyhow::anyhow!("TUI terminal was already transferred"))
    }

    /// Restores the terminal explicitly; dropping the guard repeats the best-effort restore.
    pub(crate) fn restore(&mut self) {
        if !self.active {
            return;
        }
        self.active = false;
        if let Some(terminal) = self.terminal.as_mut() {
            let _ = terminal.show_cursor();
        }
        let mut stdout = io::stdout();
        let _ = execute!(stdout, Show, DisableBracketedPaste, LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

impl Drop for TuiLifecycle {
    fn drop(&mut self) {
        self.restore();
    }
}
