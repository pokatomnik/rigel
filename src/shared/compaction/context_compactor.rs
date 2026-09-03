use std::sync::Arc;

use rig::{
    Agent,
    completion::{Chat, CompletionModel},
    message::Message,
};
use tokio::sync::Mutex;

use crate::{
    prompts::summarization::summarization,
    shared::history::{
        chat_history::{ChatHistory, HistoryUpdate},
        history_persistence::HistoryPersistence,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompactionKind {
    Manual,
    Automatic,
}

pub(crate) struct CompactionResult {
    kind: CompactionKind,
    compacted: bool,
}

impl CompactionResult {
    pub(crate) fn kind(&self) -> CompactionKind {
        self.kind
    }

    pub(crate) fn compacted(&self) -> bool {
        self.compacted
    }
}

/// Replaces a conversation with the result of the existing summarization prompt.
pub(crate) struct ContextCompactor<'a, CM, P>
where
    CM: CompletionModel,
    P: HistoryPersistence,
{
    agent: &'a Arc<Mutex<Agent<CM>>>,
    history: &'a ChatHistory<P>,
}

impl<'a, CM, P> ContextCompactor<'a, CM, P>
where
    CM: CompletionModel + 'static,
    P: HistoryPersistence,
{
    pub(crate) fn new(agent: &'a Arc<Mutex<Agent<CM>>>, history: &'a ChatHistory<P>) -> Self {
        Self { agent, history }
    }

    pub(crate) async fn compact(&self, kind: CompactionKind) -> anyhow::Result<CompactionResult> {
        let original_messages = self.history.snapshot().await;
        let mut messages = original_messages.clone();
        self.history.begin_compaction();
        let summarized = {
            let agent = self.agent.lock().await;
            agent.chat(summarization().to_string(), &mut messages).await
        };
        self.history.end_compaction();
        let Ok(summarized) = summarized else {
            self.history
                .update(HistoryUpdate::Replace(original_messages))
                .await?;
            return Ok(CompactionResult {
                kind,
                compacted: false,
            });
        };

        self.history
            .update(HistoryUpdate::Replace(vec![Message::assistant(summarized)]))
            .await?;
        Ok(CompactionResult {
            kind,
            compacted: true,
        })
    }
}
