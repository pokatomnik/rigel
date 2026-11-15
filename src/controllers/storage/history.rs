use std::{io::ErrorKind, path::PathBuf, pin::Pin, sync::Arc};

use anyhow::Context;
use tokio::{
    fs::{self, OpenOptions},
    io::AsyncWriteExt,
    sync::Mutex,
};

use crate::{
    controllers::storage::{
        paths::HistoryPaths, reader::LoadedHistory, record::ChatLine, writer::HistoryWriter,
    },
    entities::conversation::conversation_message::ConversationMessage,
    infrastructure::conversation::ConversationPersistence,
};

/// Append-only durable owner of one chat history family.
///
/// The active segment grows by appending records. Canonical rewrites are stored as a
/// `snapshot` record so earlier lines are never destroyed by a write. Compaction seals the
/// active segment as an immutable archive before starting a summary-only active segment.
#[derive(Clone)]
pub(crate) struct JsonlHistory {
    paths: HistoryPaths,
    state: Arc<Mutex<PersistenceState>>,
}

struct PersistenceState {
    writer: HistoryWriter,
    last_saved: Vec<ConversationMessage>,
    system_prompt: Option<String>,
    is_new: bool,
}

impl JsonlHistory {
    /// Creates a history that continues from an already loaded active segment.
    pub(crate) fn new(paths: &HistoryPaths, loaded: &LoadedHistory) -> Self {
        Self {
            paths: paths.clone(),
            state: Arc::new(Mutex::new(PersistenceState {
                writer: HistoryWriter::new(paths, loaded.next_id),
                last_saved: loaded.context.clone(),
                system_prompt: loaded.system_prompt.clone(),
                is_new: loaded.context.is_empty()
                    && loaded.archives.is_empty()
                    && loaded.system_prompt.is_none(),
            })),
        }
    }

    /// Persists the effective system prompt at the start of a brand-new history.
    pub(crate) async fn ensure_system_prompt(&self, prompt: &str) -> anyhow::Result<()> {
        if prompt.trim().is_empty() {
            return Ok(());
        }
        let mut state = self.state.lock().await;
        if state.system_prompt.is_some() {
            return Ok(());
        }
        if state.is_new {
            let message = ConversationMessage::System(prompt.to_string());
            state.writer.append_message(&message).await?;
            state.is_new = false;
        }
        state.system_prompt = Some(prompt.to_string());
        Ok(())
    }

    /// Deletes the active file and every archive after an explicit user confirmation.
    pub(crate) async fn reset(&self) -> anyhow::Result<()> {
        let mut state = self.state.lock().await;
        state.last_saved.clear();
        for path in self.family_files().await? {
            match fs::remove_file(path.as_path()).await {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => {
                    anyhow::bail!("failed to remove history '{}': {error}", path.display());
                }
            }
        }
        state.is_new = true;
        if let Some(prompt) = state.system_prompt.clone()
            && !prompt.trim().is_empty()
        {
            let message = ConversationMessage::System(prompt);
            state.writer.append_message(&message).await?;
            state.is_new = false;
        }
        Ok(())
    }

    async fn commit(&self, messages: &[ConversationMessage]) -> anyhow::Result<()> {
        let mut state = self.state.lock().await;
        match pending_change(state.last_saved.as_slice(), messages) {
            Change::Unchanged => Ok(()),
            Change::Append(suffix) => state.commit_append(suffix, messages).await,
            Change::Snapshot(replacement) => state.commit_snapshot(replacement).await,
        }
    }

    async fn commit_compaction_transition(&self, summary: String) -> anyhow::Result<()> {
        let mut state = self.state.lock().await;
        self.rotate_active(&mut state, summary.as_str()).await?;
        state.last_saved = vec![ConversationMessage::Assistant(summary)];
        state.is_new = false;
        Ok(())
    }

    async fn rotate_active(
        &self,
        state: &mut PersistenceState,
        summary: &str,
    ) -> anyhow::Result<()> {
        let active = self.paths.active();
        let archive = self.next_archive_path().await?;
        let lines = state.rotation_lines(summary)?;
        let active_exists = fs::try_exists(active.as_path())
            .await
            .with_context(|| format!("failed to inspect '{}'", active.display()))?;
        if active_exists {
            fs::rename(active.as_path(), archive.as_path())
                .await
                .with_context(|| {
                    format!(
                        "failed to seal '{}' as '{}'",
                        active.display(),
                        archive.display()
                    )
                })?;
        }
        match self.write_rotation(lines).await {
            Ok(()) => Ok(()),
            Err(error) => {
                if active_exists {
                    let _ = fs::rename(archive.as_path(), active.as_path()).await;
                }
                Err(error)
            }
        }
    }

    async fn next_archive_path(&self) -> anyhow::Result<PathBuf> {
        let mut entries = match fs::read_dir(self.paths.directory()).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Ok(self.paths.archive(0));
            }
            Err(error) => return Err(error.into()),
        };
        let mut next = 0;
        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(index) = HistoryPaths::archive_index(name.as_str()) {
                next = next.max(index.saturating_add(1));
            }
        }
        Ok(self.paths.archive(next))
    }

    async fn write_rotation(&self, lines: Vec<ChatLine>) -> anyhow::Result<()> {
        let temp = self.paths.directory().join("chat.tmp");
        let mut buffer = String::new();
        for line in &lines {
            buffer.push_str(
                serde_json::to_string(line)
                    .context("failed to serialize a compaction record")?
                    .as_str(),
            );
            buffer.push('\n');
        }
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(temp.as_path())
            .await
            .with_context(|| format!("failed to create '{}'", temp.display()))?;
        file.write_all(buffer.as_bytes())
            .await
            .with_context(|| format!("failed to write '{}'", temp.display()))?;
        file.sync_all()
            .await
            .with_context(|| format!("failed to sync '{}'", temp.display()))?;
        drop(file);
        fs::rename(temp.as_path(), self.paths.active().as_path())
            .await
            .with_context(|| {
                format!(
                    "failed to publish the new active history '{}'",
                    self.paths.active().display()
                )
            })
    }

    async fn family_files(&self) -> anyhow::Result<Vec<PathBuf>> {
        let mut entries = match fs::read_dir(self.paths.directory()).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut files = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_active = name == "chat.jsonl";
            let is_archive = HistoryPaths::archive_index(name.as_str()).is_some();
            if (is_active || is_archive) && entry.file_type().await?.is_file() {
                files.push(entry.path());
            }
        }
        Ok(files)
    }
}

impl PersistenceState {
    async fn commit_append(
        &mut self,
        suffix: Vec<ConversationMessage>,
        candidate: &[ConversationMessage],
    ) -> anyhow::Result<()> {
        for message in &suffix {
            self.writer.append_message(message).await?;
        }
        self.last_saved = candidate.to_vec();
        Ok(())
    }

    async fn commit_snapshot(&mut self, candidate: Vec<ConversationMessage>) -> anyhow::Result<()> {
        let payload = self.snapshot_payload(candidate.as_slice());
        self.writer.append_snapshot(payload.as_slice()).await?;
        self.last_saved = candidate;
        Ok(())
    }

    fn snapshot_payload(&self, candidate: &[ConversationMessage]) -> Vec<ConversationMessage> {
        let mut payload = Vec::with_capacity(candidate.len() + 1);
        if let Some(prompt) = self.system_prompt.clone()
            && !prompt.trim().is_empty()
        {
            payload.push(ConversationMessage::System(prompt));
        }
        payload.extend(candidate.iter().cloned());
        payload
    }

    fn rotation_lines(&mut self, summary: &str) -> anyhow::Result<Vec<ChatLine>> {
        let mut lines = Vec::new();
        if let Some(prompt) = self.system_prompt.clone()
            && !prompt.trim().is_empty()
        {
            let message = ConversationMessage::System(prompt);
            lines.push(ChatLine::message(self.writer.allocate_id(), &message)?);
        }
        lines.push(ChatLine::summary(self.writer.allocate_id(), summary));
        Ok(lines)
    }
}

enum Change {
    Unchanged,
    Append(Vec<ConversationMessage>),
    Snapshot(Vec<ConversationMessage>),
}

fn pending_change(last: &[ConversationMessage], next: &[ConversationMessage]) -> Change {
    if next == last {
        return Change::Unchanged;
    }
    if next.starts_with(last) {
        return Change::Append(next[last.len()..].to_vec());
    }
    Change::Snapshot(next.to_vec())
}

impl ConversationPersistence for JsonlHistory {
    fn save(
        &self,
        messages: &[ConversationMessage],
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + '_>> {
        let messages = messages.to_vec();
        Box::pin(async move { self.commit(messages.as_slice()).await })
    }

    #[rustfmt::skip]
    fn commit_compaction(&self, summary: String) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + '_>> { Box::pin(async move { self.commit_compaction_transition(summary).await }) }

    #[rustfmt::skip]
    fn reset(&self) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + '_>> { Box::pin(JsonlHistory::reset(self)) }
}

impl crate::controllers::model_runtime::history::history_persistence::HistoryPersistence
    for JsonlHistory
{
    #[cfg(test)]
    fn save<'a>(
        &'a self,
        messages: &'a [rig::message::Message],
    ) -> futures::future::BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let domain =
                crate::controllers::model_runtime::history::message_codec::from_provider(messages)?;
            self.commit(domain.as_slice()).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Change, PersistenceState, pending_change};
    use crate::controllers::storage::{
        paths::HistoryPaths,
        record::{RecordId, RecordKind},
        writer::HistoryWriter,
    };
    use crate::entities::conversation::conversation_message::ConversationMessage;

    fn user(text: &str) -> ConversationMessage {
        ConversationMessage::User(text.to_string())
    }

    #[test]
    fn unchanged_snapshot_produces_no_write() {
        let last = [user("first")];

        assert!(matches!(pending_change(&last, &last), Change::Unchanged));
    }

    #[test]
    fn extended_snapshot_appends_only_the_suffix() -> anyhow::Result<()> {
        let last = [user("first")];
        let next = [user("first"), user("second")];
        let Change::Append(suffix) = pending_change(&last, &next) else {
            anyhow::bail!("expected an append");
        };
        assert_eq!(suffix, [user("second")]);
        Ok(())
    }

    #[test]
    fn rewritten_snapshot_is_stored_as_a_replacement() -> anyhow::Result<()> {
        let last = [user("first"), user("second")];
        let next = [user("rewritten")];
        let Change::Snapshot(replacement) = pending_change(&last, &next) else {
            anyhow::bail!("expected a snapshot");
        };
        assert_eq!(replacement, next);
        Ok(())
    }

    fn rotation_state(system_prompt: Option<&str>) -> PersistenceState {
        let paths = HistoryPaths::from_directory("history-dir".into());
        PersistenceState {
            writer: HistoryWriter::new(&paths, RecordId::new(5)),
            last_saved: Vec::new(),
            system_prompt: system_prompt.map(str::to_string),
            is_new: false,
        }
    }

    #[test]
    fn rotation_writes_the_system_prompt_before_the_summary() -> anyhow::Result<()> {
        let mut state = rotation_state(Some("rules"));
        let lines = state.rotation_lines("summary")?;

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].id, RecordId::new(5));
        assert_eq!(lines[0].kind, RecordKind::Message);
        assert_eq!(lines[1].id, RecordId::new(6));
        assert_eq!(lines[1].kind, RecordKind::Summary);
        assert_eq!(lines[1].text.as_deref(), Some("summary"));
        Ok(())
    }

    #[test]
    fn rotation_without_a_system_prompt_writes_only_the_summary() -> anyhow::Result<()> {
        let mut state = rotation_state(None);
        let lines = state.rotation_lines("summary")?;

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].id, RecordId::new(5));
        assert_eq!(lines[0].kind, RecordKind::Summary);
        Ok(())
    }

    #[test]
    fn snapshot_payload_keeps_the_system_prompt_ahead_of_the_candidate() {
        let state = rotation_state(Some("rules"));
        let candidate = vec![user("hello")];

        assert_eq!(
            state.snapshot_payload(candidate.as_slice()),
            [
                ConversationMessage::System("rules".to_string()),
                user("hello"),
            ]
        );
    }
}
