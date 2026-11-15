use std::sync::atomic::Ordering;

use crate::entities::{
    agent_command::{command::AgentCommand, queue::QueuedPrompt},
    agent_event::{event::AgentEvent, status::GoalView, status::NoticeLevel},
};

use super::super::workflow::Chat;

impl Chat {
    pub(super) async fn run_turn_cycle(&self, initial: String) -> anyhow::Result<bool> {
        let mut next = Some(initial);
        while let Some(prompt) = next {
            self.begin_turn();
            let result = self.run_prompt_interruptible(prompt).await;
            self.finish_turn();
            if let Err(error) = result {
                self.notice(NoticeLevel::Error, error.to_string());
                self.clear_queue().await;
                return Ok(self.exit_requested.load(Ordering::SeqCst));
            }
            if self.exit_requested.load(Ordering::SeqCst) {
                self.clear_queue().await;
                return Ok(true);
            }
            if self.stop_requested.load(Ordering::SeqCst) {
                self.clear_queue().await;
                return Ok(false);
            }
            next = self.pop_queue().await;
        }
        Ok(false)
    }

    fn begin_turn(&self) {
        self.stop_requested.store(false, Ordering::SeqCst);
        self.exit_requested.store(false, Ordering::SeqCst);
        if let Some(cancellation) = self.cancellation.as_ref() {
            cancellation.reset();
        }
        self.emit_activity(crate::entities::agent_event::status::AgentActivity::Running);
    }

    fn finish_turn(&self) {
        self.emit_activity(crate::entities::agent_event::status::AgentActivity::Idle);
    }

    async fn run_prompt_interruptible(
        &self,
        prompt: String,
    ) -> anyhow::Result<Option<crate::entities::goal::goal_state::GoalReport>> {
        let turn = self.run_prompt(prompt);
        tokio::pin!(turn);
        loop {
            tokio::select! {
                result = &mut turn => {
                    return result;
                }
                command = self.next_agent_command() => {
                    let Some(command) = command else {
                        self.request_stop().await;
                        continue;
                    };
                    self.handle_turn_command(command).await?;
                }
            }
        }
    }

    /// Runs the goal loop while remaining interruptible by Stop and Exit.
    pub(crate) async fn run_goal(&self, goal: String) -> anyhow::Result<()> {
        self.activate_goal(goal.clone())?;
        self.emit(AgentEvent::GoalChanged(GoalView::active(goal.clone())));
        let mut prompt = goal;
        loop {
            if let Some(report) = self.run_prompt_interruptible(prompt).await? {
                self.emit(AgentEvent::GoalChanged(GoalView::completed(
                    report.as_str(),
                )));
                return Ok(());
            }
            if self.stop_requested.load(Ordering::SeqCst)
                || self.exit_requested.load(Ordering::SeqCst)
            {
                self.abandon_goal()?;
                return Ok(());
            }
            prompt = self.next_goal_prompt()?;
        }
    }

    async fn handle_turn_command(&self, command: AgentCommand) -> anyhow::Result<()> {
        match command {
            AgentCommand::SubmitPrompt { text } => self.enqueue_prompt(text).await,
            AgentCommand::UseSkill { .. } => self.notice(
                NoticeLevel::Info,
                "Skills are available after the agent stops.",
            ),
            AgentCommand::Stop => self.request_stop().await,
            AgentCommand::Exit => {
                if !self.exit_requested.swap(true, Ordering::SeqCst) {
                    self.cancel_interactions().await;
                    self.request_stop().await;
                }
            }
            AgentCommand::ResolvePermission {
                correlation_id,
                decision,
            } => {
                if let Some(broker) = self.interaction_broker.as_ref()
                    && !broker.resolve_permission(correlation_id, decision).await
                {
                    self.notice(
                        NoticeLevel::Warning,
                        "The permission request is no longer active.",
                    );
                }
            }
            AgentCommand::AnswerQuestion {
                correlation_id,
                answer,
            } => {
                if let Some(broker) = self.interaction_broker.as_ref()
                    && !broker.resolve_question(correlation_id, answer).await
                {
                    self.notice(NoticeLevel::Warning, "The question is no longer active.");
                }
            }
            AgentCommand::CancelQuestion { correlation_id } => {
                if let Some(broker) = self.interaction_broker.as_ref()
                    && !broker.cancel_question(correlation_id).await
                {
                    self.notice(NoticeLevel::Warning, "The question is no longer active.");
                }
            }
            AgentCommand::Compact
            | AgentCommand::NewConversation
            | AgentCommand::SelectModel { .. }
            | AgentCommand::StartGoal { .. }
            | AgentCommand::RetryPersistence
            | AgentCommand::RetryModelCatalog => self.notice(
                NoticeLevel::Info,
                "This command is available after the agent stops.",
            ),
        }
        Ok(())
    }

    async fn enqueue_prompt(&self, text: String) {
        let prompts = {
            let mut queue = self.queue.lock().await;
            queue.push(text);
            queue.prompts().to_vec()
        };
        self.emit(AgentEvent::QueueChanged(prompts));
    }

    async fn pop_queue(&self) -> Option<String> {
        let (prompt, prompts) = {
            let mut queue = self.queue.lock().await;
            let prompt = queue.pop_front()?;
            (prompt, queue.prompts().to_vec())
        };
        self.emit(AgentEvent::QueueChanged(prompts));
        Some(prompt.text().to_string())
    }

    pub(super) async fn clear_queue(&self) {
        let cleared = {
            let mut queue = self.queue.lock().await;
            queue.clear()
        };
        if !cleared.is_empty() {
            self.emit(AgentEvent::QueueChanged(Vec::<QueuedPrompt>::new()));
        }
    }
}
