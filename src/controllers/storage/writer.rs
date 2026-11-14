use std::path::PathBuf;

use anyhow::Context;
use tokio::{fs::OpenOptions, io::AsyncWriteExt};

use crate::{
    controllers::storage::{
        paths::HistoryPaths,
        record::{ChatLine, RecordId},
    },
    entities::conversation::conversation_message::ConversationMessage,
};

/// Appends durable records to the active JSONL segment without rewriting earlier lines.
pub(crate) struct HistoryWriter {
    active_path: PathBuf,
    next_id: RecordId,
}

impl HistoryWriter {
    /// Creates a writer that continues identifiers from an already loaded history.
    pub(crate) fn new(paths: &HistoryPaths, next_id: RecordId) -> Self {
        Self {
            active_path: paths.active(),
            next_id,
        }
    }

    /// Allocates the identifier for a record that the caller will build itself.
    pub(crate) fn allocate_id(&mut self) -> RecordId {
        let id = self.next_id;
        self.next_id = self.next_id.next();
        id
    }

    /// Appends one provider message record.
    pub(crate) async fn append_message(
        &mut self,
        message: &ConversationMessage,
    ) -> anyhow::Result<()> {
        let line = ChatLine::message(self.next_id, message)?;
        self.append(line).await
    }

    /// Appends one complete replacement snapshot.
    pub(crate) async fn append_snapshot(
        &mut self,
        messages: &[ConversationMessage],
    ) -> anyhow::Result<()> {
        let line = ChatLine::snapshot(self.next_id, messages)?;
        self.append(line).await
    }

    async fn append(&mut self, line: ChatLine) -> anyhow::Result<()> {
        let encoded = serde_json::to_string(&line).context("failed to serialize history record")?;
        let mut buffer = encoded.into_bytes();
        buffer.push(b'\n');
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.active_path.as_path())
            .await
            .with_context(|| {
                format!(
                    "failed to open active history '{}'",
                    self.active_path.display()
                )
            })?;
        file.write_all(buffer.as_slice()).await.with_context(|| {
            format!(
                "failed to append to active history '{}'",
                self.active_path.display()
            )
        })?;
        file.flush().await.with_context(|| {
            format!(
                "failed to flush active history '{}'",
                self.active_path.display()
            )
        })?;
        self.next_id = self.next_id.next();
        Ok(())
    }
}
