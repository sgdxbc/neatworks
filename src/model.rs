use derive_more::From;

#[derive(Debug, From)]
pub struct EventSource<M>(pub tokio::sync::mpsc::UnboundedReceiver<M>);

impl<M> EventSource<M> {
    pub async fn next(&mut self) -> crate::Result<M> {
        self.0
            .recv()
            .await
            .ok_or(crate::err!("unexpected source closing"))
    }

    pub async fn option_next(&mut self) -> Option<M> {
        self.0.recv().await
    }
}

#[derive(Debug, From)]
pub struct EventSender<M>(pub tokio::sync::mpsc::UnboundedSender<M>);

impl<M> Clone for EventSender<M> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<M> EventSender<M> {
    pub fn send(&self, message: M) -> crate::Result<()> {
        self.0
            .send(message)
            .map_err(|_| crate::err!("unexpected event channel closing"))
    }
}

pub fn event_channel<M>() -> (EventSender<M>, EventSource<M>) {
    let channel = tokio::sync::mpsc::unbounded_channel();
    (EventSender(channel.0), EventSource(channel.1))
}

pub trait Message
where
    Self: Clone + Send + Sync + 'static,
{
}

impl<M> Message for M where M: Clone + Send + Sync + 'static {}

#[async_trait::async_trait]
pub trait Transport<M>
where
    Self: Clone + Send + Sync + 'static,
{
    fn addr(&self) -> crate::Addr;

    async fn send_to(&self, destination: crate::Addr, message: M) -> crate::Result<()>
    where
        M: Message;

    async fn send_to_all(
        &self,
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
