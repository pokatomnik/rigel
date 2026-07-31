use std::{path::Path, sync::Mutex as StdMutex};

use anyhow::Context;
use rig::message::Message;
use tokio::{
    fs,
    sync::{Mutex, mpsc},
    task::JoinHandle,
};

const HISTORY_DIRECTORY: &str = ".rigel";
const HISTORY_FILE: &str = "history.json";

pub(crate) struct History {
    save_tx: StdMutex<Option<mpsc::UnboundedSender<String>>>,
    writer_task: Mutex<Option<JoinHandle<anyhow::Result<()>>>>,
}

impl History {
    pub async fn bootstrap() -> anyhow::Result<(Self, Vec<Message>)> {
        let history_directory = dirs::home_dir()
            .context("failed to determine the user home directory")?
            .join(HISTORY_DIRECTORY);
        fs::create_dir_all(history_directory.as_path())
            .await
            .with_context(|| {
                format!(
                    "failed to create history directory '{}'",
                    history_directory.display()
                )
            })?;

        let history_path = history_directory.join(HISTORY_FILE);
        let messages = match fs::read_to_string(history_path.as_path()).await {
            Ok(json) => Self::deserialize(json.as_str()).with_context(|| {
                format!(
                    "failed to parse chat history from '{}'",
                    history_path.display()
                )
            })?,
            Err(_) => Vec::new(),
        };

        let (save_tx, save_rx) = mpsc::unbounded_channel();

        let writer_task =
            tokio::spawn(async move { Self::write_queued(history_path.as_path(), save_rx).await });

        Ok((
            Self {
                save_tx: StdMutex::new(Some(save_tx)),
                writer_task: Mutex::new(Some(writer_task)),
            },
            messages,
        ))
    }

    fn serialize(messages: &[&Message]) -> serde_json::Result<String> {
        serde_json::to_string(messages)
    }

    fn deserialize(json: &str) -> serde_json::Result<Vec<Message>> {
        serde_json::from_str(json)
    }

    pub fn save(&self, messages: &[&Message]) -> anyhow::Result<()> {
        let json = Self::serialize(messages)?;
        self.save_tx
            .lock()
            .map_err(|_| anyhow::anyhow!("chat history save queue lock is poisoned"))?
            .as_ref()
            .context("chat history writer is not running")?
            .send(json)
            .map_err(|_| anyhow::anyhow!("chat history writer is not running"))
    }

    pub async fn shutdown(&self) -> anyhow::Result<()> {
        let save_tx = self
            .save_tx
            .lock()
            .map_err(|_| anyhow::anyhow!("chat history save queue lock is poisoned"))?
            .take();
        drop(save_tx);

        let writer_task = self
            .writer_task
            .lock()
            .await
            .take()
            .context("chat history writer has already been shut down")?;

        writer_task
            .await
            .context("chat history writer task failed")??;

        Ok(())
    }

    async fn write_queued(
        path: &Path,
        mut save_rx: mpsc::UnboundedReceiver<String>,
    ) -> anyhow::Result<()> {
        while let Some(json) = save_rx.recv().await {
            fs::write(path, json).await.map_err(|error| {
                anyhow::anyhow!(
                    "failed to write chat history to '{}': {error}",
                    path.display()
                )
            })?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicUsize, Ordering},
    };

    use rig::message::Message;
    use tokio::sync::{Mutex, mpsc};

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
    fn save_serializes_before_queuing() {
        let (save_tx, mut save_rx) = mpsc::unbounded_channel();
        let history = History {
            save_tx: StdMutex::new(Some(save_tx)),
            writer_task: Mutex::new(None),
        };
        let messages = [Message::user("Hello")];
        let message_refs = messages.iter().collect::<Vec<_>>();

        history.save(&message_refs).expect("save should be queued");

        let json = save_rx
            .try_recv()
            .expect("serialized history should be queued");
        assert_eq!(
            json,
            History::serialize(&message_refs).expect("messages should serialize")
        );
    }

    #[test]
    fn save_reports_closed_writer() {
        let (save_tx, save_rx) = mpsc::unbounded_channel();
        drop(save_rx);
        let history = History {
            save_tx: StdMutex::new(Some(save_tx)),
            writer_task: Mutex::new(None),
        };

        let error = history.save(&[]).expect_err("closed writer should fail");

        assert_eq!(error.to_string(), "chat history writer is not running");
    }

    #[tokio::test]
    async fn shutdown_drains_queued_saves() {
        let (save_tx, mut save_rx) = mpsc::unbounded_channel();
        let saved_count = Arc::new(AtomicUsize::new(0));
        let writer_saved_count = saved_count.clone();
        let writer_task = tokio::spawn(async move {
            while save_rx.recv().await.is_some() {
                writer_saved_count.fetch_add(1, Ordering::Relaxed);
            }
            Ok(())
        });
        let history = History {
            save_tx: StdMutex::new(Some(save_tx)),
            writer_task: Mutex::new(Some(writer_task)),
        };

        history.save(&[]).expect("save should be queued");
        history.shutdown().await.expect("shutdown should finish");

        assert_eq!(saved_count.load(Ordering::Relaxed), 1);
    }
}
