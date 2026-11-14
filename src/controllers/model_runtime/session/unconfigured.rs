use std::{future::Future, pin::Pin};

use crate::{
    entities::conversation::{
        context_usage::ContextUsage,
        turn::{CompactionKind, CompactionResult, TurnStatus},
    },
    infrastructure::model::ModelSession,
};

/// Session used only while Rigel has no model with a configured context limit.
///
/// It never contacts a provider. The UI opens the palette in the "not configured" state and
/// offers only exit; this session keeps the command loop alive so exit and shutdown still work.
pub(crate) struct UnconfiguredModelSession;

impl UnconfiguredModelSession {
    /// Creates the provider-free placeholder session.
    pub(crate) const fn new() -> Self {
        Self
    }
}

impl ModelSession for UnconfiguredModelSession {
    fn run_turn(
        &self,
        _prompt: String,
        _context_usage: ContextUsage,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<TurnStatus>> + Send + '_>> {
        Box::pin(async {
            anyhow::bail!("Rigel is not configured: add `context_size` to a model in the config")
        })
    }

    fn compact(
        &self,
        _kind: CompactionKind,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<CompactionResult>> + Send + '_>> {
        Box::pin(async { anyhow::bail!("Rigel is not configured") })
    }

    fn max_context_tokens(&self) -> Option<u64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::UnconfiguredModelSession;
    use crate::{
        entities::conversation::{context_usage::ContextUsage, turn::CompactionKind},
        infrastructure::model::ModelSession,
    };

    #[tokio::test]
    async fn every_model_operation_reports_the_configuration_problem() {
        let session = UnconfiguredModelSession::new();
        let turn = session
            .run_turn("hello".to_string(), ContextUsage::new(None, None))
            .await;
        let compact = session.compact(CompactionKind::Manual).await;

        assert!(turn.is_err_and(|error| error.to_string().contains("context_size")));
        assert!(compact.is_err());
        assert_eq!(session.max_context_tokens(), None);
    }
}
