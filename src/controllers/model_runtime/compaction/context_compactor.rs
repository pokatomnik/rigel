use std::sync::Arc;

use futures::future::BoxFuture;
use rig::{
    Agent,
    completion::{Chat, CompletionModel},
};
use tokio::sync::Mutex;

use crate::{
    controllers::model_runtime::history::message_codec::to_provider,
    entities::conversation::conversation_message::ConversationMessage,
    infrastructure::{conversation::ConversationSummarizer, prompts::PromptProvider},
    shared::terminal::loader::WithLoader,
};

/// Adapts Rig chat summarization to the provider-neutral conversation boundary.
pub(crate) struct RigContextSummarizer<'a, CM>
where
    CM: CompletionModel,
{
    agent: &'a Arc<Mutex<Agent<CM>>>,
    prompt_provider: &'a dyn PromptProvider,
}

impl<'a, CM> RigContextSummarizer<'a, CM>
where
    CM: CompletionModel + 'static,
{
    /// Creates a Rig summarizer from an agent and prompt provider without owning history policy.
    pub(crate) fn new(
        agent: &'a Arc<Mutex<Agent<CM>>>,
        prompt_provider: &'a dyn PromptProvider,
    ) -> Self {
        Self {
            agent,
            prompt_provider,
        }
    }
}

impl<CM> ConversationSummarizer for RigContextSummarizer<'_, CM>
where
    CM: CompletionModel + 'static,
{
    fn summarize<'a>(
        &'a self,
        messages: &'a [ConversationMessage],
    ) -> BoxFuture<'a, anyhow::Result<String>> {
        Box::pin(async move {
            let mut provider_messages = to_provider(messages)?;
            let agent = self.agent.lock().await;
            agent
                .chat(self.prompt_provider.summarization(), &mut provider_messages)
                .with_spinner()
                .await
                .map_err(Into::into)
        })
    }
}
