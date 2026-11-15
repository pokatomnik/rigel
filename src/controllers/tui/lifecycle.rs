use std::{
    io::{self, IsTerminal, Stdout},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

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
    active: Arc<AtomicBool>,
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
            restore_terminal_best_effort();
            return Err(error.into());
        }
        // Keyboard enhancement is intentionally not requested. Crossterm 0.29
        // does not parse the Kitty base-layout key or associated text, so full
        // CSI-u reporting breaks Ctrl shortcuts and text input on non-Latin
        // layouts. Legacy encoding keeps Ctrl+C/Q/K layout-independent, while
        // Alt+Enter still reports a newline as ESC CR.
        match Terminal::new(CrosstermBackend::new(stdout)) {
            Ok(terminal) => {
                let active = Arc::new(AtomicBool::new(true));
                install_panic_hook(active.clone());
                Ok(Self {
                    terminal: Some(terminal),
                    active,
                })
            }
            Err(error) => {
                restore_terminal_best_effort();
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
        if !self.active.swap(false, Ordering::SeqCst) {
            return;
        }
        if let Some(terminal) = self.terminal.as_mut() {
            let _ = terminal.show_cursor();
        }
        restore_terminal_best_effort();
    }
}

fn restore_terminal_best_effort() {
    let _ = execute!(
        io::stdout(),
        Show,
        DisableBracketedPaste,
        LeaveAlternateScreen
    );
    let _ = disable_raw_mode();
}

fn install_panic_hook(active: Arc<AtomicBool>) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if active.swap(false, Ordering::SeqCst) {
            restore_terminal_best_effort();
        }
        previous(info);
    }));
}

impl Drop for TuiLifecycle {
    fn drop(&mut self) {
        self.restore();
    }
}
