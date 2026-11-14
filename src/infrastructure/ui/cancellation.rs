/// Cooperative cancellation shared by one interactive run and its child sessions.
pub(crate) trait RunCancellation: Send + Sync {
    /// Requests a soft stop at the next safe boundary.
    fn cancel(&self);

    /// Clears a previous request before a new turn starts.
    fn reset(&self);

    /// Reports whether a soft stop is currently requested.
    fn is_cancelled(&self) -> bool;
}
