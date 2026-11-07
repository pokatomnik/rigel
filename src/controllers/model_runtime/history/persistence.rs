use std::{
    io::{self, ErrorKind},
    path::{Path, PathBuf},
};

use anyhow::Context;
use rig::message::Message;

use crate::{
    entities::conversation::conversation_message::ConversationMessage,
    infrastructure::conversation::ConversationPersistence, shared::hash::sha256,
};

use super::message_codec::{from_provider, to_provider};

use tokio::{fs, sync::Semaphore};

const RIGEL_DIRECTORY: &str = ".rigel";
const HISTORY_FILE_NAME: &str = "history.json";

/// Persists chat history to a single JSON file per working directory.
///
/// Each working directory owns a hash-named subdirectory of `.rigel` that holds its
/// `history.json`. All file reads and writes are serialized through a one-permit semaphore, so at
/// most one history I/O operation runs at a time.
pub(crate) struct History {
    io_semaphore: Semaphore,
    history_path: PathBuf,
}

impl History {
    pub(crate) fn history_file_path() -> anyhow::Result<PathBuf> {
        let current_dir = std::env::current_dir()?.to_string_lossy().to_string();
        let current_dir_hash = hash_directory(current_dir.as_bytes());

        let home_dir_path = std::env::home_dir()
            .ok_or_else(|| anyhow::anyhow!("failed to determine the user home directory"))?;
        let rigel_directory_path = home_dir_path.join(RIGEL_DIRECTORY);
        let history_file_path = rigel_directory_path
            .join(current_dir_hash)
            .join(HISTORY_FILE_NAME);

        Ok(history_file_path)
    }

    pub async fn bootstrap() -> anyhow::Result<(Self, Vec<ConversationMessage>)> {
        let history_path = Self::history_file_path()?;
        let history_directory = history_path
            .parent()
            .context("history file path has no parent directory")?;
        fs::create_dir_all(history_directory)
            .await
            .with_context(|| {
                format!(
                    "failed to create history directory '{}'",
                    history_directory.display()
                )
            })?;

        let io_semaphore = Semaphore::new(1);
        let messages = {
            let _read_permit = io_semaphore
                .acquire()
                .await
                .context("failed to acquire the chat history read permit")?;
            let read_result = fs::read_to_string(history_path.as_path()).await;
            from_provider(&Self::messages_from_read_result(
                read_result,
                history_path.as_path(),
            )?)?
        };

        Ok((
            Self {
                io_semaphore,
                history_path,
            },
            messages,
        ))
    }

    fn serialize(messages: &[&Message]) -> serde_json::Result<String> {
        serde_json::to_string_pretty(messages)
    }

    fn deserialize(json: &str) -> serde_json::Result<Vec<Message>> {
        serde_json::from_str(json)
    }

    fn messages_from_read_result(
        read_result: io::Result<String>,
        history_path: &Path,
    ) -> anyhow::Result<Vec<Message>> {
        match read_result {
            Ok(json) => Self::deserialize(json.as_str()).with_context(|| {
                format!(
                    "failed to parse chat history from '{}'",
                    history_path.display()
                )
            }),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(error).with_context(|| {
                format!(
                    "failed to read chat history from '{}'",
                    history_path.display()
                )
            }),
        }
    }

    pub async fn save_provider(&self, messages: &[Message]) -> anyhow::Result<()> {
        let references = messages.iter().collect::<Vec<_>>();
        let json = Self::serialize(&references).context("failed to serialize chat history")?;
        let _write_permit = self
            .io_semaphore
            .acquire()
            .await
            .context("failed to acquire the chat history write permit")?;

        fs::write(self.history_path.as_path(), json.as_str())
            .await
            .with_context(|| {
                format!(
                    "failed to write chat history to '{}'",
                    self.history_path.display()
                )
            })
    }
}

impl ConversationPersistence for History {
    fn save(
        &self,
        messages: &[ConversationMessage],
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + '_>> {
        let provider_messages = match to_provider(messages) {
            Ok(messages) => messages,
            Err(error) => return Box::pin(async move { Err(error) }),
        };
        Box::pin(async move { History::save_provider(self, &provider_messages).await })
    }
}

fn hash_directory(value: &[u8]) -> String {
    sha256(value)
}

#[cfg(test)]
mod tests {
    use std::{
        io::{self, ErrorKind},
        path::Path,
    };

    use anyhow::ensure;
    use rig::message::Message;

    use super::History;

    #[test]
    fn messages_round_trip_through_json() -> anyhow::Result<()> {
        let messages = [Message::user("Hello"), Message::assistant("Hi")];
        let message_refs = messages.iter().collect::<Vec<_>>();

        let json = History::serialize(&message_refs)?;

        let restored = History::deserialize(json.as_str())?;
        let restored_refs = restored.iter().collect::<Vec<_>>();
        let restored_json = History::serialize(&restored_refs)?;

        ensure!(restored_json == json, "round-trip JSON changed");
        Ok(())
    }

    #[test]
    fn legacy_history_fixture_round_trips_without_rewriting_messages() -> anyhow::Result<()> {
        let json = include_str!("fixtures/history.json");
        let messages = History::deserialize(json)?;
        let domain = super::super::message_codec::from_provider(messages.as_slice())?;
        let restored = super::super::message_codec::to_provider(domain.as_slice())?;
        assert_eq!(
            serde_json::to_value(restored)?,
            serde_json::to_value(messages)?
        );
        Ok(())
    }

    #[test]
    fn invalid_json_is_rejected() -> anyhow::Result<()> {
        match History::deserialize("not json") {
            Ok(_) => anyhow::bail!("invalid JSON was accepted"),
            Err(error) => ensure!(error.is_syntax(), "unexpected JSON error: {error}"),
        }

        Ok(())
    }

    #[test]
    fn missing_history_returns_empty_messages() -> anyhow::Result<()> {
        let messages = History::messages_from_read_result(
            Err(io::Error::from(ErrorKind::NotFound)),
            Path::new("history.json"),
        )?;

        ensure!(messages.is_empty(), "missing history was not empty");
        Ok(())
    }

    #[test]
    fn history_read_error_keeps_context() -> anyhow::Result<()> {
        let result = History::messages_from_read_result(
            Err(io::Error::new(ErrorKind::PermissionDenied, "denied")),
            Path::new("history.json"),
        );
        let error = match result {
            Ok(_) => anyhow::bail!("permission error was ignored"),
            Err(error) => error,
        };

        ensure!(
            format!("{error:#}") == "failed to read chat history from 'history.json': denied",
            "unexpected read error: {error:#}"
        );
        Ok(())
    }
}
