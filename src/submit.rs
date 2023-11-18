use tokio::sync::{mpsc, oneshot};

#[derive(Debug, Clone)]
pub struct Handle<T, U>(pub mpsc::UnboundedSender<(T, oneshot::Sender<U>)>);

impl<T, U> Handle<T, U> {
    pub async fn submit(&self, op: T) -> crate::Result<U> {
        let chan = oneshot::channel();
        self.0
            .send((op, chan.0))
            .map_err(|_| crate::err!("unexpected submit channel closing"))?;
        Ok(chan.1.await?)
    }
}

pub type Receiver<T, U> = mpsc::UnboundedReceiver<(T, oneshot::Sender<U>)>;
