use tokio::sync::{mpsc, oneshot};

#[async_trait::async_trait]
pub trait App
where
    Self: Send + 'static,
{
    async fn execute(&mut self, op: &[u8]) -> Vec<u8>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Null;

#[async_trait::async_trait]
impl App for Null {
    async fn execute(&mut self, _: &[u8]) -> Vec<u8> {
        Default::default()
    }
}

#[derive(Debug, Clone)]
pub struct AppHandle<T, U>(pub mpsc::UnboundedSender<(T, oneshot::Sender<U>)>);

impl<T, U> AppHandle<T, U> {
    pub async fn invoke(&self, op: T) -> crate::Result<U> {
        let chan = oneshot::channel();
        self.0.send((op, chan.0)).map_err(|_| "send fail")?;
        Ok(chan.1.await?)
    }
}
