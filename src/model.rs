#[async_trait::async_trait]
pub trait EventSource<E> {
    async fn next(&mut self) -> Option<E>;
}

pub trait Message
where
    Self: Clone + Send + Sync + 'static,
{
}

impl<M> Message for M where M: Clone + Send + Sync + 'static {}

#[async_trait::async_trait]
pub trait Transport<M> {
    fn addr(&self) -> crate::Addr;

    async fn send_to(&mut self, destination: crate::Addr, message: M) -> crate::Result<()>
    where
        M: Message;

    async fn send_to_all(
        &mut self,
        destinations: impl Iterator<Item = crate::Addr> + Send,
        message: M,
    ) -> crate::Result<()>
    where
        M: Message,
    {
        for destination in destinations {
            if destination == self.addr() {
                crate::bail!("unexpected loopback message")
            }
            self.send_to(destination, message.clone()).await?
        }
        Ok(())
    }
}
