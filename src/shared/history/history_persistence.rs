use std::sync::Arc;

use futures::future::BoxFuture;
use rig::message::Message;

use super::history::History;

pub(crate) trait HistoryPersistence: Send + Sync + 'static {
    fn save<'a>(&'a self, messages: &'a [Message]) -> BoxFuture<'a, anyhow::Result<()>>;
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct NonPersistentHistory;

impl HistoryPersistence for NonPersistentHistory {
    fn save<'a>(&'a self, _messages: &'a [Message]) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async { Ok(()) })
    }
}

impl HistoryPersistence for History {
    fn save<'a>(&'a self, messages: &'a [Message]) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let references = messages.iter().collect::<Vec<_>>();
            History::save(self, &references).await
        })
    }
}

impl<P> HistoryPersistence for Arc<P>
where
    P: HistoryPersistence,
{
    fn save<'a>(&'a self, messages: &'a [Message]) -> BoxFuture<'a, anyhow::Result<()>> {
        self.as_ref().save(messages)
    }
}
