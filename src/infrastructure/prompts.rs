use std::{future::Future, pin::Pin};

/// Supplies model-facing text while keeping static assets in `src/prompts`.
pub(crate) trait PromptProvider: Send + Sync {
    /// Returns the chat system prompt for the selected tool set.
    fn chat_system(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<String>> + Send + '_>>;

    /// Returns the reduced subagent system prompt.
    fn subagent_system(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<String>> + Send + '_>>;

    /// Adds the autonomous-task/report contract to a task.
    fn task(&self, task: &str) -> String;

    /// Returns the summarization request.
    fn summarization(&self) -> String;

    /// Builds a goal continuation request.
    fn goal_follow_up(&self, goal: &str) -> String;

    /// Builds recovery guidance for a failed provider stream.
    fn recovery(&self, error: &str, attempt: usize, max_attempts: usize) -> String;

    /// Builds recovery guidance when the model omitted its final answer.
    fn missing_answer_recovery(&self, attempt: usize, max_attempts: usize) -> String;

    /// Builds recovery guidance for an unresolved tool failure.
    fn unresolved_tool_recovery(&self, attempt: usize, max_attempts: usize) -> String;
}
