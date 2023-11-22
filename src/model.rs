use derive_more::From;

#[derive(Debug, From)]
pub struct EventSource<M>(tokio::sync::mpsc::UnboundedReceiver<M>);

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
pub struct EventSender<M>(tokio::sync::mpsc::UnboundedSender<M>);

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

/// A thin wrapper around Tokio's unbounded MPSC channel.
/// 
/// Wrapped to integrate with `crate::Result`.
pub fn event_channel<M>() -> (EventSender<M>, EventSource<M>) {
    let channel = tokio::sync::mpsc::unbounded_channel();
    (EventSender(channel.0), EventSource(channel.1))
}

#[derive(Debug, From)]
pub struct PromiseSender<T>(tokio::sync::oneshot::Sender<T>);

pub type PromiseSource<T> = tokio::sync::oneshot::Receiver<T>;

pub fn promise_channel<T>() -> (PromiseSender<T>, PromiseSource<T>) {
    let chan = tokio::sync::oneshot::channel();
    (PromiseSender(chan.0), chan.1)
}

impl<T> PromiseSender<T> {
    pub fn resolve(self, value: T) -> crate::Result<()> {
        self.0
            .send(value)
            .map_err(|_| crate::err!("unexpected return channel closing"))
    }
}

pub type SubmitHandle<T, U> = EventSender<(T, PromiseSender<U>)>;

impl<T, U> SubmitHandle<T, U> {
    pub async fn submit(&self, op: T) -> crate::Result<U> {
        let chan = promise_channel();
        self.send((op, chan.0))?;
        Ok(chan.1.await?)
    }
}

pub type SubmitSource<T, U> = EventSource<(T, PromiseSender<U>)>;

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
