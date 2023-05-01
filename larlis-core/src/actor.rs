use std::mem::replace;

use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

pub trait State<'message> {
    type Message;

    fn update(&mut self, message: Self::Message);
}

pub trait SharedClone: Clone {}

pub struct Drive<M> {
    sender: UnboundedSender<M>,
    receiver: UnboundedReceiver<M>,
}

pub struct DriveState<M>(UnboundedSender<M>);

impl<M> Clone for DriveState<M> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<M> SharedClone for DriveState<M> {}

impl<M> State<'_> for DriveState<M> {
    type Message = M;

    fn update(&mut self, message: Self::Message) {
        if self.0.send(message).is_err() {
            //
        }
    }
}

impl<M> Default for Drive<M> {
    fn default() -> Self {
        let (sender, receiver) = unbounded_channel();
        Self { sender, receiver }
    }
}

impl<M> Drive<M> {
    pub fn state(&self) -> DriveState<M> {
        DriveState(self.sender.clone())
    }

    pub async fn recv(&mut self) -> Option<M> {
        // may need rethinking
        let sender = replace(&mut self.sender, unbounded_channel().0);
        let weak_sender = sender.downgrade();
        drop(sender);
        let message = self.receiver.recv().await;
        if let Some(sender) = weak_sender.upgrade() {
            self.sender = sender;
        }
        message
    }

    pub async fn run(mut self, state: &mut impl State<'_, Message = M>) {
        drop(self.sender);
        while let Some(message) = self.receiver.recv().await {
            state.update(message)
        }
    }
}