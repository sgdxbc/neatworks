use tokio::sync::oneshot;

use crate::model::{EventSender, EventSource};

#[derive(Debug)]
pub struct Promise<T>(oneshot::Sender<T>);

pub type PromiseSource<T> = oneshot::Receiver<T>;

pub fn promise_channel<T>() -> (Promise<T>, PromiseSource<T>) {
    let chan = oneshot::channel();
    (Promise(chan.0), chan.1)
}

impl<T> Promise<T> {
    pub fn resolve(self, value: T) -> crate::Result<()> {
        self.0
            .send(value)
            .map_err(|_| crate::err!("unexpected return channel closing"))
    }
}

pub type Handle<T, U> = EventSender<(T, Promise<U>)>;

impl<T, U> Handle<T, U> {
    pub async fn submit(&self, op: T) -> crate::Result<U> {
        let chan = promise_channel();
        self.send((op, chan.0))?;
        Ok(chan.1.await?)
    }
}

pub type Receiver<T, U> = EventSource<(T, Promise<U>)>;
