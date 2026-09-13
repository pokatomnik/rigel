use crate::{
    contracts::prompts::PromptProvider,
    prompts::{
        goal::goal_follow_up,
        subagent::task_prompt,
        system::{chat_system::chat_system_prompt, subagent_system::subagent_system_prompt},
    },
};

/// Runtime adapter that composes the static prompt assets for provider clients.
pub(crate) struct StaticPromptProvider;

impl PromptProvider for StaticPromptProvider {
    fn chat_system(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send + '_>>
    {
        Box::pin(async { Ok(chat_system_prompt().await) })
    }

    fn subagent_system(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send + '_>>
    {
        Box::pin(async { Ok(subagent_system_prompt().await) })
    }

    fn task(&self, task: &str) -> String {
        task_prompt(task.to_owned())
    }

    fn summarization(&self) -> String {
        crate::prompts::summarization::summarization().to_owned()
    }

    fn goal_follow_up(&self, goal: &str) -> String {
        goal_follow_up(goal)
    }
}
