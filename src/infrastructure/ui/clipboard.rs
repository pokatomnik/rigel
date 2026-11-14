/// Copies text to the operating system clipboard without exposing a clipboard crate.
pub(crate) trait Clipboard: Send + Sync {
    /// Replaces the clipboard contents with the exact payload.
    fn copy(&self, text: &str) -> anyhow::Result<()>;
}
