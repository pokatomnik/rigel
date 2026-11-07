use std::{future::Future, pin::Pin, sync::Arc};

use crate::{
    entities::{
        conversation::{
            context_usage::ContextUsage,
            turn::{CompactionKind, TurnStatus},
        },
        model::model_session_kind::ModelSessionKind,
        tool::tool_entities::{SubagentReport, SubagentTask},
    },
    infrastructure::{
        interaction::ChatIO, model::ModelSessionFactory, prompts::PromptProvider,
        tools::subagent::SubagentRunner,
    },
};

/// Runs delegated tasks through provider-neutral model, prompt, and interaction ports.
pub(crate) struct RunSubagent {
    factory: Arc<dyn ModelSessionFactory>,
    prompts: Arc<dyn PromptProvider>,
    io: Arc<dyn ChatIO>,
}

impl RunSubagent {
    /// Creates a task runner from its required application capabilities.
    pub(crate) fn new(
        factory: Arc<dyn ModelSessionFactory>,
        prompts: Arc<dyn PromptProvider>,
        io: Arc<dyn ChatIO>,
    ) -> Self {
        Self {
            factory,
            prompts,
            io,
        }
    }

    async fn run_task(&self, task: SubagentTask) -> anyhow::Result<SubagentReport> {
        let session = self
            .factory
            .create(ModelSessionKind::Subagent, task.selected_model().cloned())
            .await?;
        let mut usage = ContextUsage::new(None, session.max_context_tokens());
        let prompt = self.prompts.task(&task.task);
        let status = session.run_turn(prompt, usage).await?;
        usage = status.context_usage();
        self.compact_if_needed(session.as_ref(), &status, &mut usage)
            .await?;
        self.report(status)
    }

    async fn compact_if_needed(
        &self,
        session: &dyn crate::infrastructure::model::ModelSession,
        status: &TurnStatus,
        usage: &mut ContextUsage,
    ) -> anyhow::Result<()> {
        if !matches!(status, TurnStatus::Complete { .. }) || !usage.should_compact() {
            return Ok(());
        }
        self.io.eprintln_red("Compacting context...");
        let result = session.compact(CompactionKind::Automatic).await?;
        if result.kind() == CompactionKind::Automatic && result.compacted() {
            *usage = ContextUsage::new(None, usage.max_context_tokens());
            self.io.eprintln_red("Context compacted.");
        } else {
            self.io.eprintln_red("Compacting failed.");
        }
        Ok(())
    }

    fn report(&self, status: TurnStatus) -> anyhow::Result<SubagentReport> {
        match status {
            TurnStatus::Complete { output, .. } => Ok(SubagentReport { report: output }),
            TurnStatus::RecoveryStopped { error, .. } => {
                anyhow::bail!("subagent recovery stopped: {error}")
            }
        }
    }
}

impl SubagentRunner for RunSubagent {
    fn run(
        &self,
        task: SubagentTask,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<SubagentReport>> + Send + '_>> {
        Box::pin(self.run_task(task))
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use anyhow::Result;
    use futures::future::BoxFuture;

    use super::RunSubagent;
    use crate::{
        entities::model::model_session_kind::ModelSessionKind,
        entities::{
            application::input_outcome::InputOutcome,
            conversation::{
                context_usage::ContextUsage,
                conversation_message::ConversationMessage,
                turn::{CompactionKind, CompactionResult, TurnStatus},
            },
            skill::definition::SkillDescriptor,
            tool::tool_entities::{SubagentReport, SubagentTask},
        },
        infrastructure::{
            interaction::ChatIO,
            model::{ModelSession, ModelSessionFactory},
            prompts::PromptProvider,
            tools::subagent::SubagentRunner,
        },
    };

    struct FakeIo {
        messages: Mutex<Vec<String>>,
    }

    impl FakeIo {
        fn new() -> Self {
            Self {
                messages: Mutex::new(Vec::new()),
            }
        }
    }

    impl ChatIO for FakeIo {
        fn readline(&self, _: ContextUsage) -> anyhow::Result<InputOutcome> {
            Ok(InputOutcome::Eof)
        }

        fn editor(&self) -> anyhow::Result<String> {
            Ok(String::new())
        }

        fn print(&self, _: &str) {}
        fn eprintln(&self, _: &str) {}
        fn eprintln_gray(&self, _: &str) {}

        fn eprintln_red(&self, message: &str) {
            let _ = self
                .messages
                .lock()
                .map(|mut messages| messages.push(message.to_string()));
        }

        fn eprintln_orange(&self, _: &str) {}
        fn select_skill(&self, _: &[SkillDescriptor]) -> anyhow::Result<Option<usize>> {
            Ok(None)
        }
        fn display_history(&self, _: &[ConversationMessage]) {}
    }

    struct FakeFactory {
        statuses: Arc<tokio::sync::Mutex<VecDeque<TurnStatus>>>,
        created: Arc<AtomicUsize>,
        selected_models:
            Arc<Mutex<Vec<Option<crate::entities::selected_model::selection::SelectedModel>>>>,
        run_turns: Arc<AtomicUsize>,
        compactions: Arc<AtomicUsize>,
        compacted: bool,
    }

    impl ModelSessionFactory for FakeFactory {
        fn create(
            &self,
            kind: ModelSessionKind,
            selected_model: Option<crate::entities::selected_model::selection::SelectedModel>,
        ) -> BoxFuture<'_, anyhow::Result<Box<dyn ModelSession>>> {
            let statuses = self.statuses.clone();
            let created = self.created.clone();
            let selected_models = self.selected_models.clone();
            let run_turns = self.run_turns.clone();
            let compactions = self.compactions.clone();
            let compacted = self.compacted;
            Box::pin(async move {
                anyhow::ensure!(kind == ModelSessionKind::Subagent);
                selected_models
                    .lock()
                    .map_err(|_| anyhow::anyhow!("selected model lock poisoned"))?
                    .push(selected_model);
                created.fetch_add(1, Ordering::SeqCst);
                let status = statuses
                    .lock()
                    .await
                    .pop_front()
                    .ok_or_else(|| anyhow::anyhow!("no fake turn configured"))?;
                Ok(Box::new(FakeSession {
                    status,
                    run_turns,
                    compactions,
                    compacted,
                }) as Box<dyn ModelSession>)
            })
        }
    }

    struct FakeSession {
        status: TurnStatus,
        run_turns: Arc<AtomicUsize>,
        compactions: Arc<AtomicUsize>,
        compacted: bool,
    }

    impl ModelSession for FakeSession {
        fn run_turn(
            &self,
            _: String,
            _: ContextUsage,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = anyhow::Result<TurnStatus>> + Send + '_>,
        > {
            self.run_turns.fetch_add(1, Ordering::SeqCst);
            let status = self.status.clone();
            Box::pin(async move { Ok(status) })
        }

        fn compact(
            &self,
            kind: CompactionKind,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = anyhow::Result<CompactionResult>> + Send + '_>,
        > {
            self.compactions.fetch_add(1, Ordering::SeqCst);
            let compacted = self.compacted;
            Box::pin(async move { Ok(CompactionResult::new(kind, compacted)) })
        }

        fn max_context_tokens(&self) -> Option<u64> {
            Some(1_000)
        }
    }

    fn completed(output: &str, usage: ContextUsage) -> TurnStatus {
        TurnStatus::Complete {
            output: output.to_string(),
            context_usage: usage,
        }
    }

    struct RunHarness {
        runner: RunSubagent,
        io: Arc<FakeIo>,
        created: Arc<AtomicUsize>,
        selected_models:
            Arc<Mutex<Vec<Option<crate::entities::selected_model::selection::SelectedModel>>>>,
        turns: Arc<AtomicUsize>,
        compactions: Arc<AtomicUsize>,
    }

    fn runner(statuses: Vec<TurnStatus>, compacted: bool) -> RunHarness {
        let io = Arc::new(FakeIo::new());
        let created = Arc::new(AtomicUsize::new(0));
        let selected_models = Arc::new(Mutex::new(Vec::new()));
        let turns = Arc::new(AtomicUsize::new(0));
        let compactions = Arc::new(AtomicUsize::new(0));
        let factory = Arc::new(FakeFactory {
            statuses: Arc::new(tokio::sync::Mutex::new(statuses.into())),
            created: created.clone(),
            selected_models: selected_models.clone(),
            run_turns: turns.clone(),
            compactions: compactions.clone(),
            compacted,
        });
        let prompts: Arc<dyn PromptProvider> =
            Arc::new(crate::prompts::provider::StaticPromptProvider);
        RunHarness {
            runner: RunSubagent::new(factory, prompts, io.clone()),
            io,
            created,
            selected_models,
            turns,
            compactions,
        }
    }

    fn task() -> SubagentTask {
        SubagentTask::new("delegate".to_string())
    }

    #[tokio::test]
    async fn successful_runs_create_fresh_sessions_for_each_call() -> Result<()> {
        let harness = runner(
            vec![
                completed("first", ContextUsage::new(Some(10), Some(1_000))),
                completed("second", ContextUsage::new(Some(20), Some(1_000))),
            ],
            false,
        );
        let first_model = crate::entities::selected_model::selection::SelectedModel::new(
            "parent-a".to_string(),
            Some(1_000),
        );
        let second_model = crate::entities::selected_model::selection::SelectedModel::new(
            "parent-b".to_string(),
            Some(2_000),
        );
        let first = harness
            .runner
            .run(task().with_selected_model(first_model.clone()))
            .await?;
        let second = harness
            .runner
            .run(task().with_selected_model(second_model.clone()))
            .await?;
        assert_eq!(
            first,
            SubagentReport {
                report: "first".to_string()
            }
        );
        assert_eq!(
            second,
            SubagentReport {
                report: "second".to_string()
            }
        );
        assert_eq!(harness.created.load(Ordering::SeqCst), 2);
        assert_eq!(harness.turns.load(Ordering::SeqCst), 2);
        assert_eq!(
            *harness
                .selected_models
                .lock()
                .map_err(|_| anyhow::anyhow!("selected model lock poisoned"))?,
            vec![Some(first_model), Some(second_model)]
        );
        Ok(())
    }

    #[tokio::test]
    async fn recovery_stopped_turn_returns_the_original_actionable_error() {
        let status = TurnStatus::RecoveryStopped {
            error: "tool recovery exhausted".to_string(),
            context_usage: ContextUsage::new(Some(900), Some(1_000)),
        };
        let harness = runner(vec![status], true);
        let error = harness.runner.run(task()).await.err();
        assert!(error.is_some_and(|error| {
            error
                .to_string()
                .contains("subagent recovery stopped: tool recovery exhausted")
        }));
        assert_eq!(harness.turns.load(Ordering::SeqCst), 1);
        assert_eq!(harness.compactions.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn complete_turn_at_threshold_compacts_once_without_continuing() -> Result<()> {
        let usage = ContextUsage::new(Some(800), Some(1_000));
        let harness = runner(vec![completed("done", usage)], true);
        let report = harness.runner.run(task()).await?;
        assert_eq!(report.report, "done");
        assert_eq!(harness.turns.load(Ordering::SeqCst), 1);
        assert_eq!(harness.compactions.load(Ordering::SeqCst), 1);
        assert!(harness.io.messages.lock().is_ok_and(|messages| {
            messages.as_slice() == ["Compacting context...", "Context compacted."]
        }));
        Ok(())
    }

    #[tokio::test]
    async fn complete_turn_below_threshold_does_not_compact() -> Result<()> {
        let usage = ContextUsage::new(Some(799), Some(1_000));
        let harness = runner(vec![completed("done", usage)], true);
        harness.runner.run(task()).await?;
        assert_eq!(harness.turns.load(Ordering::SeqCst), 1);
        assert_eq!(harness.compactions.load(Ordering::SeqCst), 0);
        Ok(())
    }
}
