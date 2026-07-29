pub(crate) trait Controller<T> {
    async fn handle(&self, deps: T) -> anyhow::Result<()>;
}
