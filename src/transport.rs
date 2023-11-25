pub trait Message
where
    Self: Clone + Send + Sync + 'static,
{
}

impl<M> Message for M where M: Clone + Send + Sync + 'static {}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Addr {
    Socket(std::net::SocketAddr),
    Untyped(String),
}

#[async_trait::async_trait]
pub trait Transport<M>
where
    Self: Clone + Send + Sync + 'static,
{
    fn addr(&self) -> Addr;

    async fn send_to(&self, destination: Addr, message: M) -> crate::Result<()>
    where
        M: Message;

    async fn send_to_all(
        &self,
        destinations: impl Iterator<Item = Addr> + Send,
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
