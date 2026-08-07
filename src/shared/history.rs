use std::{
    io::{self, ErrorKind},
    path::{Path, PathBuf},
};

use anyhow::Context;
use rig::message::Message;
use tokio::{fs, sync::Semaphore};

use crate::{
    shared::consts::{HISTORY_FILE_NAME, RIGEL_DIRECTORY},
    tools::revision::sha256,
};

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
        let current_dir_hash = sha256(current_dir.as_bytes());

        let home_dir_path = std::env::home_dir()
            .ok_or_else(|| anyhow::anyhow!("failed to determine the user home directory"))?;
        let rigel_directory_path = home_dir_path.join(RIGEL_DIRECTORY);
        let history_file_path = rigel_directory_path
            .join(current_dir_hash)
            .join(HISTORY_FILE_NAME);

        Ok(history_file_path)
    }

    pub async fn bootstrap() -> anyhow::Result<(Self, Vec<Message>)> {
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
            Self::messages_from_read_result(read_result, history_path.as_path())?
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

    pub async fn save(&self, messages: &[&Message]) -> anyhow::Result<()> {
        let json = Self::serialize(messages).context("failed to serialize chat history")?;
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
