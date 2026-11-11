/// Describes the result of reading one line from an interactive input source.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum InputOutcome {
    /// A line was read and normalized by the input adapter.
    Text(String),
    /// The input source reached EOF.
    Eof,
    /// The user interrupted the input operation.
    Cancelled,
}
