use std::{future::Future, pin::Pin, sync::Arc};

use crate::{
    contracts::{
        application::ChatRunner,
        model::{ModelSessionFactory, ModelSessionKind},
    },
    entities::application::chat_request::ChatRequest,
};

use super::super::{
    agent::{agent_config::AgentConfig, dependencies::AgentDependencies},
    chat::{
        chat::Chat as RuntimeChat,
        history::{chat_history::ChatHistory, history::History},
    },
    model_session::RigModelSessionFactory,
};

/// Runs the provider-specific interactive chat runtime behind `ChatRunner`.
pub(crate) struct RuntimeChatRunner {
    config: AgentConfig,
    dependencies: Arc<AgentDependencies>,
}

impl RuntimeChatRunner {
    /// Creates a runtime runner from composition-root-selected capabilities.
    pub(crate) fn new(config: AgentConfig, dependencies: Arc<AgentDependencies>) -> Self {
        Self {
            config,
            dependencies,
        }
    }

    async fn run_session(&self) -> anyhow::Result<()> {
        let (history, messages) = History::bootstrap().await?;
        let chat_history = Arc::new(ChatHistory::new(messages, Arc::new(history)));
        let factory = Arc::new(RigModelSessionFactory::new(
            self.config.clone(),
            self.dependencies.clone(),
            chat_history.clone(),
        ));
        let session = factory.create(ModelSessionKind::Chat, None).await?;
        let runtime_chat =
            RuntimeChat::from_session(session, self.dependencies.terminal_io(), chat_history)
                .with_model_session_factory(factory)
                .await
                .with_goal_state(self.dependencies.goal_state()?);
        runtime_chat.run().await
    }
}

impl ChatRunner for RuntimeChatRunner {
    fn run(&self, _request: ChatRequest) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + '_>> {
        Box::pin(self.run_session())
    }
}
