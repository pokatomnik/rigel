use std::sync::Arc;

#[cfg(test)]
use futures::future::BoxFuture;
#[cfg(test)]
use rig::message::Message;

pub(crate) trait HistoryPersistence: Send + Sync + 'static {
    #[cfg(test)]
    fn save<'a>(&'a self, messages: &'a [Message]) -> BoxFuture<'a, anyhow::Result<()>>;
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct NonPersistentHistory;

impl HistoryPersistence for NonPersistentHistory {
    #[cfg(test)]
    fn save<'a>(&'a self, _messages: &'a [Message]) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async { Ok(()) })
    }
}

impl<P> HistoryPersistence for Arc<P>
where
    P: HistoryPersistence,
{
    #[cfg(test)]
    fn save<'a>(&'a self, messages: &'a [Message]) -> BoxFuture<'a, anyhow::Result<()>> {
        self.as_ref().save(messages)
    }
}
