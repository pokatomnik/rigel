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
        let guard = CompactionGuard::begin(history);
        let summary = summarizer.summarize(original.as_slice()).await;
        guard.finish();
        match summary {
            Ok(text) => self.commit_summary(kind, text, history).await,
            Err(error) => {
                self.restore_original(original, history).await?;
                Err(error)
            }
        }
    }

    async fn commit_summary(
        &self,
        kind: CompactionKind,
        summary: String,
        history: &dyn ConversationHistory,
    ) -> anyhow::Result<CompactionResult> {
        history.commit_compaction(summary).await?;
        Ok(CompactionResult::new(kind, true))
    }

    async fn restore_original(
        &self,
        original: Vec<ConversationMessage>,
        history: &dyn ConversationHistory,
    ) -> anyhow::Result<()> {
        history.update(HistoryUpdate::Replace(original)).await
    }
}

/// Releases the compaction marker even when the future is cancelled.
struct CompactionGuard<'a> {
    history: &'a dyn ConversationHistory,
    released: bool,
}

impl<'a> CompactionGuard<'a> {
    fn begin(history: &'a dyn ConversationHistory) -> Self {
        history.begin_compaction();
        Self {
            history,
            released: false,
        }
    }

    fn finish(mut self) {
        self.release();
    }

    fn release(&mut self) {
        if !self.released {
            self.history.end_compaction();
            self.released = true;
        }
    }
}

impl Drop for CompactionGuard<'_> {
    fn drop(&mut self) {
        self.release();
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
    async fn summarizer_failure_restores_original_and_returns_the_cause() -> anyhow::Result<()> {
        let original = original_messages();
        let history = Arc::new(FakeHistory::new(original.clone()));
        let summarizer = FakeSummarizer::new(Err("model failed".to_string()), history.clone());
        let result = compactor()
            .compact(CompactionKind::Automatic, &summarizer, history.as_ref())
            .await;
        let error = match result {
            Ok(_) => anyhow::bail!("summarizer failure was reported as success"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("model failed"));
        assert_eq!(history.messages.lock().await.as_slice(), original);
        assert!(summarizer.observed_compaction.load(Ordering::SeqCst));
        assert!(!history.is_compaction_in_progress());
        assert_eq!(history.begins.load(Ordering::SeqCst), 1);
        assert_eq!(history.ends.load(Ordering::SeqCst), 1);
        Ok(())
    }

    #[tokio::test]
    async fn empty_summary_is_committed_as_an_explicit_risk() -> anyhow::Result<()> {
        let history = Arc::new(FakeHistory::new(original_messages()));
        let summarizer = FakeSummarizer::new(Ok(String::new()), history.clone());
        let result = compactor()
            .compact(CompactionKind::Manual, &summarizer, history.as_ref())
            .await?;

        assert!(result.compacted());
        assert_eq!(
            history.messages.lock().await.as_slice(),
            [ConversationMessage::Assistant(String::new())]
        );
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
