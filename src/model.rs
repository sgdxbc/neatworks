#[async_trait::async_trait]
pub trait EventSource<E> {
    async fn next(&mut self) -> Option<E>;
}

#[async_trait::async_trait]
pub trait Transport<M> {
    async fn send_to(&mut self, destination: crate::Addr, message: M) -> crate::Result<()>
    where
        M: Clone + Send + Sync + 'static;

    async fn send_to_all(
        &mut self,
        destinations: impl Iterator<Item = crate::Addr> + Send,
        message: M,
    ) -> crate::Result<()>
    where
        M: Clone + Send + Sync + 'static,
    {
        for destination in destinations {
            self.send_to(destination, message.clone()).await?
        }
        Ok(())
    }
}
