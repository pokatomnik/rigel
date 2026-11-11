use futures::future::BoxFuture;

use crate::{
    entities::conversation::{
        conversation_message::ConversationMessage,
        turn::{CompactionKind, CompactionResult, HistoryUpdate},
    },
    infrastructure::conversation::{
        ContextCompaction, ConversationHistory, ConversationSummarizer,
    },
};

/// Applies transactional context-compaction policy to a conversation history.
pub(crate) struct CompactContext;

impl ContextCompaction for CompactContext {
    fn compact<'a>(
        &'a self,
        kind: CompactionKind,
        summarizer: &'a dyn ConversationSummarizer,
        history: &'a dyn ConversationHistory,
    ) -> BoxFuture<'a, anyhow::Result<CompactionResult>> {
        Box::pin(self.compact_history(kind, summarizer, history))
    }
}

impl CompactContext {
    async fn compact_history(
        &self,
        kind: CompactionKind,
        summarizer: &dyn ConversationSummarizer,
        history: &dyn ConversationHistory,
    ) -> anyhow::Result<CompactionResult> {
        let original = history.snapshot().await;
        history.begin_compaction();
        let summary = summarizer.summarize(original.as_slice()).await;
        history.end_compaction();
        match summary {
            Ok(text) => self.commit_summary(kind, text, history).await,
            Err(_) => self.restore_original(kind, original, history).await,
        }
    }

    async fn commit_summary(
        &self,
        kind: CompactionKind,
        summary: String,
        history: &dyn ConversationHistory,
    ) -> anyhow::Result<CompactionResult> {
        history
            .update(HistoryUpdate::Replace(vec![
                ConversationMessage::Assistant(summary),
            ]))
            .await?;
        Ok(CompactionResult::new(kind, true))
    }

    async fn restore_original(
        &self,
        kind: CompactionKind,
        original: Vec<ConversationMessage>,
        history: &dyn ConversationHistory,
    ) -> anyhow::Result<CompactionResult> {
        history.update(HistoryUpdate::Replace(original)).await?;
        Ok(CompactionResult::new(kind, false))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    use tokio::sync::Mutex;

    use super::*;

    struct FakeHistory {
        messages: Mutex<Vec<ConversationMessage>>,
        compacting: AtomicBool,
        begins: AtomicUsize,
        ends: AtomicUsize,
        fail_updates: AtomicBool,
    }

    impl FakeHistory {
        fn new(messages: Vec<ConversationMessage>) -> Self {
            Self {
                messages: Mutex::new(messages),
                compacting: AtomicBool::new(false),
                begins: AtomicUsize::new(0),
                ends: AtomicUsize::new(0),
                fail_updates: AtomicBool::new(false),
            }
        }
    }

    impl ConversationHistory for FakeHistory {
        fn snapshot(&self) -> BoxFuture<'_, Vec<ConversationMessage>> {
            Box::pin(async { self.messages.lock().await.clone() })
        }

        fn update(&self, update: HistoryUpdate) -> BoxFuture<'_, anyhow::Result<()>> {
            Box::pin(async move {
                if self.fail_updates.load(Ordering::SeqCst) {
                    anyhow::bail!("history persistence failed");
                }
                let HistoryUpdate::Replace(messages) = update else {
                    anyhow::bail!("compaction expected a history replacement");
                };
                *self.messages.lock().await = messages;
                Ok(())
            })
        }

        fn begin_compaction(&self) {
            self.begins.fetch_add(1, Ordering::SeqCst);
            self.compacting.store(true, Ordering::SeqCst);
        }

        fn end_compaction(&self) {
            self.ends.fetch_add(1, Ordering::SeqCst);
            self.compacting.store(false, Ordering::SeqCst);
        }

        fn is_compaction_in_progress(&self) -> bool {
            self.compacting.load(Ordering::SeqCst)
        }
    }

    struct FakeSummarizer {
        result: Result<String, String>,
        history: Arc<FakeHistory>,
        observed_compaction: AtomicBool,
    }

    impl FakeSummarizer {
        fn new(result: Result<String, String>, history: Arc<FakeHistory>) -> Self {
            Self {
                result,
                history,
                observed_compaction: AtomicBool::new(false),
            }
        }
    }

    impl ConversationSummarizer for FakeSummarizer {
        fn summarize<'a>(
            &'a self,
            _messages: &'a [ConversationMessage],
        ) -> BoxFuture<'a, anyhow::Result<String>> {
            Box::pin(async move {
                self.observed_compaction
                    .store(self.history.is_compaction_in_progress(), Ordering::SeqCst);
                let result = self.result.clone();
                if result.is_err() {
                    *self.history.messages.lock().await = vec![ConversationMessage::User(
                        "history hook mutation".to_string(),
                    )];
                }
                result.map_err(anyhow::Error::msg)
            })
        }
    }

    fn original_messages() -> Vec<ConversationMessage> {
        vec![ConversationMessage::User("request".to_string())]
    }

    fn compactor() -> CompactContext {
        CompactContext
    }

    #[tokio::test]
    async fn success_replaces_history_and_balances_compaction_hooks() -> anyhow::Result<()> {
        let history = Arc::new(FakeHistory::new(original_messages()));
        let summarizer = FakeSummarizer::new(Ok("summary".to_string()), history.clone());
        let result = compactor()
            .compact(CompactionKind::Manual, &summarizer, history.as_ref())
            .await?;

        assert!(result.compacted());
        assert_eq!(result.kind(), CompactionKind::Manual);
        assert_eq!(
            history.messages.lock().await.as_slice(),
            [ConversationMessage::Assistant("summary".to_string())]
        );
        assert!(summarizer.observed_compaction.load(Ordering::SeqCst));
        assert!(!history.is_compaction_in_progress());
        assert_eq!(history.begins.load(Ordering::SeqCst), 1);
        assert_eq!(history.ends.load(Ordering::SeqCst), 1);
        Ok(())
    }

    #[tokio::test]
    async fn summarizer_failure_restores_original_and_reports_not_compacted() -> anyhow::Result<()>
    {
        let original = original_messages();
        let history = Arc::new(FakeHistory::new(original.clone()));
        let summarizer = FakeSummarizer::new(Err("model failed".to_string()), history.clone());
        let result = compactor()
            .compact(CompactionKind::Automatic, &summarizer, history.as_ref())
            .await?;

        assert!(!result.compacted());
        assert_eq!(result.kind(), CompactionKind::Automatic);
        assert_eq!(history.messages.lock().await.as_slice(), original);
        assert!(summarizer.observed_compaction.load(Ordering::SeqCst));
        assert!(!history.is_compaction_in_progress());
        assert_eq!(history.begins.load(Ordering::SeqCst), 1);
        assert_eq!(history.ends.load(Ordering::SeqCst), 1);
        Ok(())
    }

    #[tokio::test]
    async fn persistence_failure_propagates_after_summarization() {
        let history = Arc::new(FakeHistory::new(original_messages()));
        history.fail_updates.store(true, Ordering::SeqCst);
        let summarizer = FakeSummarizer::new(Ok("summary".to_string()), history.clone());

        let result = compactor()
            .compact(CompactionKind::Manual, &summarizer, history.as_ref())
            .await;

        assert!(result.is_err());
        assert!(summarizer.observed_compaction.load(Ordering::SeqCst));
        assert!(!history.is_compaction_in_progress());
        assert_eq!(history.begins.load(Ordering::SeqCst), 1);
        assert_eq!(history.ends.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn restore_persistence_failure_is_returned() {
        let history = Arc::new(FakeHistory::new(original_messages()));
        history.fail_updates.store(true, Ordering::SeqCst);
        let summarizer = FakeSummarizer::new(Err("model failed".to_string()), history.clone());

        let result = compactor()
            .compact(CompactionKind::Automatic, &summarizer, history.as_ref())
            .await;

        assert!(result.is_err());
        assert!(summarizer.observed_compaction.load(Ordering::SeqCst));
        assert!(!history.is_compaction_in_progress());
        assert_eq!(history.begins.load(Ordering::SeqCst), 1);
        assert_eq!(history.ends.load(Ordering::SeqCst), 1);
    }
}
