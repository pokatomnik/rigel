use std::sync::Arc;

use rig::{
    Agent,
    completion::{Chat, CompletionModel},
    message::Message,
};
use tokio::sync::Mutex;

use crate::{
    adapters::model_runtime::chat::history::{
        chat_history::{ChatHistory, HistoryUpdate},
        history_persistence::HistoryPersistence,
    },
    adapters::model_runtime::prompt_provider::StaticPromptProvider,
    contracts::prompts::PromptProvider,
    entities::conversation::turn::{CompactionKind, CompactionResult},
    shared::terminal::loader::WithLoader,
};

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
            agent
                .chat(StaticPromptProvider.summarization(), &mut messages)
                .with_spinner()
                .await
        };
        self.history.end_compaction();
        let Ok(summarized) = summarized else {
            self.history
                .update(HistoryUpdate::Replace(original_messages))
                .await?;
            return Ok(CompactionResult::new(kind, false));
        };

        self.history
            .update(HistoryUpdate::Replace(vec![Message::assistant(summarized)]))
            .await?;
        Ok(CompactionResult::new(kind, true))
    }
}
