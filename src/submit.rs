use tokio::sync::oneshot;

use crate::model::{EventSender, EventSource};

#[derive(Debug)]
pub struct Return<T>(oneshot::Sender<T>);

impl<T> Return<T> {
    pub fn send(self, value: T) -> crate::Result<()> {
        self.0
            .send(value)
            .map_err(|_| crate::err!("unexpected return channel closing"))
    }
}

pub type Handle<T, U> = EventSender<(T, Return<U>)>;

impl<T, U> Handle<T, U> {
    pub async fn submit(&self, op: T) -> crate::Result<U> {
        let chan = oneshot::channel();
        self.send((op, Return(chan.0)))?;
        Ok(chan.1.await?)
    }
}

pub type Receiver<T, U> = EventSource<(T, Return<U>)>;
