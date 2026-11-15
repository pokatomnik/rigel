use std::sync::atomic::{AtomicBool, Ordering};

use crate::infrastructure::ui::cancellation::RunCancellation;

/// Atomic soft-stop flag shared by one chat run and every child session.
#[derive(Debug, Default)]
pub(crate) struct TurnCancellation {
    cancelled: AtomicBool,
}

impl TurnCancellation {
    /// Creates a run that has not been asked to stop.
    pub(crate) fn new() -> Self {
        Self::default()
    }
}

impl RunCancellation for TurnCancellation {
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    fn reset(&self) {
        self.cancelled.store(false, Ordering::SeqCst);
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::TurnCancellation;
    use crate::infrastructure::ui::cancellation::RunCancellation;

    #[test]
    fn cancellation_is_idempotent_and_resettable() {
        let cancellation = TurnCancellation::new();
        assert!(!cancellation.is_cancelled());

        cancellation.cancel();
        cancellation.cancel();
        assert!(cancellation.is_cancelled());

        cancellation.reset();
        assert!(!cancellation.is_cancelled());
    }
}
