use flume::{Receiver, Sender};

use crate::{actor::SharedClone, State};

#[derive(Debug)]
pub struct Wire<M> {
    sender: Sender<M>,
    receiver: Receiver<M>,
}

pub struct WireState<M>(Sender<M>);

impl<M> Clone for WireState<M> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<M> SharedClone for WireState<M> {}

impl<M> State<M> for WireState<M> {
    fn update(&mut self, message: M) {
        if self.0.send(message).is_err() {
            //
        }
    }
}

impl<M> Default for Wire<M> {
    fn default() -> Self {
        let (sender, receiver) = flume::unbounded();
        Self { sender, receiver }
    }
}

#[derive(Debug)]
pub struct Drive<M>(Receiver<M>);

impl<M> From<Wire<M>> for Drive<M> {
    fn from(value: Wire<M>) -> Self {
        Self(value.receiver)
    }
}

impl<M> Wire<M> {
    pub fn state(&self) -> WireState<M> {
        WireState(self.sender.clone())
    }
}

impl<M> Drive<M> {
    pub async fn recv(&mut self) -> Option<M> {
        self.0.recv_async().await.ok()
    }

    pub async fn run(&mut self, mut state: impl State<M>) {
        while let Ok(message) = self.0.recv_async().await {
            state.update(message)
        }
    }
}
