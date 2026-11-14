use crate::entities::{
    agent_command::command::AgentCommand,
    agent_event::{
        event::AgentEvent,
        status::{AgentActivity, NoticeLevel},
    },
    conversation::turn::CompactionKind,
};

use super::super::workflow::Chat;

impl Chat {
    pub(crate) async fn run_commands(&self) -> anyhow::Result<()> {
        loop {
            let Some(command) = self.next_agent_command().await else {
                return Ok(());
            };
            if !self.handle_agent_command(command).await? {
                return Ok(());
            }
        }
    }

    async fn handle_agent_command(&self, command: AgentCommand) -> anyhow::Result<bool> {
        match command {
            AgentCommand::SubmitPrompt { text } => self.run_prompt_command(text).await,
            AgentCommand::UseSkill { skill } => match self.skill_instructions(&skill).await {
                Ok(text) => self.run_prompt_command(text).await,
                Err(error) => {
                    self.notice(NoticeLevel::Error, format!("Could not load skill: {error}"));
                    Ok(true)
                }
            },
            AgentCommand::Stop => {
                self.request_stop().await;
                Ok(true)
            }
            AgentCommand::Compact => {
                self.emit_activity(AgentActivity::Compacting);
                self.emit(AgentEvent::CompactionStarted {
                    kind: CompactionKind::Manual,
                });
                let result = self.compact_context().await;
                self.emit_activity(AgentActivity::Idle);
                self.report_compaction(result);
                Ok(true)
            }
            AgentCommand::NewConversation => {
                if let Err(error) = self.handle_new().await {
                    self.notice(
                        NoticeLevel::Error,
                        format!("Could not clear history: {error}"),
                    );
                }
                Ok(true)
            }
            AgentCommand::SelectModel { model } => {
                if let Err(error) = self.handle_change_model(Some(model)).await {
                    self.notice(
                        NoticeLevel::Error,
                        format!("Could not change model: {error}"),
                    );
                }
                Ok(true)
            }
            AgentCommand::StartGoal { goal } => {
                if let Err(error) = self.run_goal(goal).await {
                    self.notice(NoticeLevel::Error, format!("Goal failed: {error}"));
                    let _ = self.abandon_goal();
                }
                self.emit_activity(AgentActivity::Idle);
                Ok(true)
            }
            AgentCommand::ResolvePermission {
                correlation_id,
                decision,
            } => {
                self.resolve_permission(correlation_id, decision).await;
                Ok(true)
            }
            AgentCommand::AnswerQuestion {
                correlation_id,
                answer,
            } => {
                self.resolve_answer(correlation_id, answer).await;
                Ok(true)
            }
            AgentCommand::CancelQuestion { correlation_id } => {
                self.cancel_question(correlation_id).await;
                Ok(true)
            }
            AgentCommand::RetryPersistence | AgentCommand::RetryModelCatalog => {
                self.notice(NoticeLevel::Info, "There is nothing to retry.");
                Ok(true)
            }
            AgentCommand::Exit => {
                self.cancel_interactions().await;
                self.clear_queue().await;
                Ok(false)
            }
        }
    }

    async fn run_prompt_command(&self, text: String) -> anyhow::Result<bool> {
        if text.trim().is_empty() {
            return Ok(true);
        }
        let exit = self.run_turn_cycle(text).await?;
        self.emit_activity(AgentActivity::Idle);
        Ok(!exit)
    }

    pub(crate) fn emit_activity(&self, activity: AgentActivity) {
        self.emit(AgentEvent::ActivityChanged(activity));
    }

    pub(crate) fn notice(&self, level: NoticeLevel, text: impl Into<String>) {
        self.emit(AgentEvent::Notice(
            crate::entities::agent_event::status::Notice::new(level, text),
        ));
    }

    pub(super) async fn request_stop(&self) {
        if self
            .stop_requested
            .swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            return;
        }
        if let Some(cancellation) = self.cancellation.as_ref() {
            cancellation.cancel();
        }
        self.emit_activity(AgentActivity::Stopping);
        self.clear_queue().await;
    }

    async fn resolve_permission(
        &self,
        correlation_id: u64,
        decision: crate::entities::tool_confirm_result::outcome::ToolConfirmResult,
    ) {
        let resolved = match self.interaction_broker.as_ref() {
            Some(broker) => broker.resolve_permission(correlation_id, decision).await,
            None => false,
        };
        if !resolved {
            self.notice(
                NoticeLevel::Warning,
                "The permission request is no longer active.",
            );
        }
    }

    async fn resolve_answer(&self, correlation_id: u64, answer: String) {
        let resolved = match self.interaction_broker.as_ref() {
            Some(broker) => broker.resolve_question(correlation_id, answer).await,
            None => false,
        };
        if !resolved {
            self.notice(NoticeLevel::Warning, "The question is no longer active.");
        }
    }

    async fn cancel_question(&self, correlation_id: u64) {
        let cancelled = match self.interaction_broker.as_ref() {
            Some(broker) => broker.cancel_question(correlation_id).await,
            None => false,
        };
        if !cancelled {
            self.notice(NoticeLevel::Warning, "The question is no longer active.");
        }
    }

    pub(super) async fn cancel_interactions(&self) {
        if let Some(broker) = self.interaction_broker.as_ref() {
            broker.cancel_all().await;
        }
    }

    fn report_compaction(&self, result: anyhow::Result<bool>) {
        match result {
            Ok(compacted) => self.emit(AgentEvent::CompactionFinished {
                kind: CompactionKind::Manual,
                compacted,
            }),
            Err(error) => {
                self.emit(AgentEvent::CompactionFailed {
                    kind: CompactionKind::Manual,
                    error: error.to_string(),
                });
                self.notice(NoticeLevel::Error, error.to_string());
            }
        }
    }
}
