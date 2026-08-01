use std::path::PathBuf;

use anyhow::Context;
use rig::message::Message;
use tokio::{fs, sync::Semaphore};

use crate::tools::revision::sha256;

const RIGEL_DIRECTORY: &str = ".rigel";

/// Persists chat history to a single JSON file per working directory.
///
/// All file reads and writes are serialized through a one-permit semaphore, so at most one
/// history I/O operation runs at a time.
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
        let history_file_path = rigel_directory_path.join(format!("{current_dir_hash}.json"));

        Ok(history_file_path)
    }

    pub async fn bootstrap() -> anyhow::Result<(Self, Vec<Message>)> {
        let history_directory = std::env::home_dir()
            .context("failed to determine the user home directory")?
            .join(RIGEL_DIRECTORY);
        fs::create_dir_all(history_directory.as_path())
            .await
            .with_context(|| {
                format!(
                    "failed to create history directory '{}'",
                    history_directory.display()
                )
            })?;

        let history_path = Self::history_file_path()?;
        let io_semaphore = Semaphore::new(1);
        let messages = {
            let _read_permit = io_semaphore
                .acquire()
                .await
                .context("failed to acquire the chat history read permit")?;
            match fs::read_to_string(history_path.as_path()).await {
                Ok(json) => Self::deserialize(json.as_str()).with_context(|| {
                    format!(
                        "failed to parse chat history from '{}'",
                        history_path.display()
                    )
                })?,
                Err(_) => Vec::new(),
            }
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

    pub async fn save(&self, messages: &[&Message]) -> anyhow::Result<()> {
        let _write_permit = self
            .io_semaphore
            .acquire()
            .await
            .context("failed to acquire the chat history write permit")?;
        let json = Self::serialize(messages)?;
        fs::write(self.history_path.as_path(), json.as_str())
            .await
            .map_err(|error| {
                anyhow::anyhow!(
                    "failed to write chat history to '{}': {error}",
                    self.history_path.display()
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rig::message::Message;
    use tokio::sync::Semaphore;

    use super::History;

    #[test]
    fn messages_round_trip_through_json() {
        let messages = [Message::user("Hello"), Message::assistant("Hi")];
        let message_refs = messages.iter().collect::<Vec<_>>();

        let json = History::serialize(&message_refs).expect("messages should serialize");
        let restored = History::deserialize(json.as_str()).expect("messages should deserialize");
        let restored_refs = restored.iter().collect::<Vec<_>>();

        assert_eq!(
            History::serialize(&restored_refs).expect("restored messages should serialize"),
            json
        );
    }

    #[test]
    fn invalid_json_is_rejected() {
        let error = History::deserialize("not json").expect_err("invalid JSON should fail");

        assert!(error.is_syntax());
    }

    #[test]
    fn io_operations_are_serialized_by_a_single_permit() {
        let history = History {
            io_semaphore: Semaphore::new(1),
            history_path: PathBuf::from("unused"),
        };

        let first_permit = history
            .io_semaphore
            .try_acquire()
            .expect("the first operation should acquire the only permit");
        assert!(
            history.io_semaphore.try_acquire().is_err(),
            "a concurrent operation must not acquire a permit while one is held"
        );
        drop(first_permit);
        assert!(
            history.io_semaphore.try_acquire().is_ok(),
            "the permit must be reusable after the operation releases it"
        );
    }
}
