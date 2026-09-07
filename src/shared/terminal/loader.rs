use std::{
    future::Future,
    io::IsTerminal,
    sync::atomic::{AtomicBool, Ordering},
};

use spinners::{Spinner, Spinners, Stream};

const LOADER_MESSAGE: &str = "Working...";

fn new_spinner() -> Spinner {
    Spinner::with_stream(Spinners::Dots, LOADER_MESSAGE.to_string(), Stream::Stderr)
}

/// Owns one spinner and makes its stop operation idempotent.
pub(crate) struct SpinnerHandle {
    spinner: Option<Spinner>,
    stopped: AtomicBool,
}

impl SpinnerHandle {
    fn start() -> Self {
        Self {
            spinner: Some(new_spinner()),
            stopped: AtomicBool::new(false),
        }
    }

    fn stop(&mut self) {
        if self.stopped.swap(true, Ordering::AcqRel) {
            return;
        }
        if let Some(mut spinner) = self.spinner.take() {
            spinner.stop_with_message(String::new());
        }
    }
}

impl Drop for SpinnerHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Owns the spinner for one asynchronous operation until its first streamed output.
///
/// `Shown` owns the concrete spinner. `Hidden` means that no terminal loader was
/// started.
pub(crate) enum SpinnerGuard {
    /// Owns a visible spinner that is stopped when the guard is dropped.
    Shown(SpinnerHandle),
    /// Does not own an active spinner.
    Hidden,
}

impl SpinnerGuard {
    /// Starts a spinner when output is interactive and visible.
    ///
    /// Non-interactive output, including tests and redirected stderr, remains
    /// untouched because a terminal loader has no useful presentation there.
    pub(crate) fn start() -> Self {
        if cfg!(test) || !std::io::stderr().is_terminal() {
            return Self::Hidden;
        }
        Self::Shown(SpinnerHandle::start())
    }

    /// Stops the spinner before the first streamed output is printed.
    pub(crate) fn hide(&mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        if let Self::Shown(spinner) = self {
            spinner.stop();
        }
    }
}

impl Drop for SpinnerGuard {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Runs an operation while an indeterminate loader is rendered on `stderr`.
///
/// The operation's result is returned unchanged, and cancelling the future drops
/// the spinner guard as well.
pub(crate) async fn with_loader<F, T>(operation: F) -> T
where
    F: Future<Output = T>,
{
    let _guard = SpinnerGuard::start();
    operation.await
}

/// Adds loader behavior to an async operation without changing its output type.
pub(crate) trait WithLoader: Future + Sized {
    /// Awaits this future while displaying an indeterminate terminal loader.
    async fn with_spinner(self) -> Self::Output;
}

impl<F> WithLoader for F
where
    F: Future,
{
    async fn with_spinner(self) -> Self::Output {
        with_loader(self).await
    }
}
