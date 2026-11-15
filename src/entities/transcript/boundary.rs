use serde::{Deserialize, Serialize};

/// Marks where the model context was replaced by a compacted summary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CompactionBoundary {
    /// File name of the immutable archive segment holding the pre-compaction history.
    pub(crate) archive: String,
}

impl CompactionBoundary {
    /// Creates a boundary that references one sealed archive segment.
    pub(crate) fn new(archive: impl Into<String>) -> Self {
        Self {
            archive: archive.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CompactionBoundary;

    #[test]
    fn boundary_keeps_the_exact_archive_name() {
        let boundary = CompactionBoundary::new("chat.0.jsonl");

        assert_eq!(boundary.archive, "chat.0.jsonl");
    }
}
