use tokio::sync::oneshot;

use crate::model::{EventSender, EventSource};

pub type Handle<T, U> = EventSender<(T, oneshot::Sender<U>)>;

impl<T, U> Handle<T, U> {
    pub async fn submit(&self, op: T) -> crate::Result<U> {
        let chan = oneshot::channel();
        self.send((op, chan.0))?;
        Ok(chan.1.await?)
    }
}

pub type Receiver<T, U> = EventSource<(T, oneshot::Sender<U>)>;
