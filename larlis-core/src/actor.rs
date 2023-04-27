use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

pub trait State<'message> {
    type Message;

    fn update(&mut self, message: Self::Message);
}

/// Lifetime-erasured version of `State<'static>`
///
/// Do not implement this, instead, implement `State<'static>` (or better,
/// implement `State<'_>`) and get this through blacket imeplementation.
/// The propose of this trait is to solve some tricky higher rank lifetime
/// error.
pub trait StateStatic {
    type Message;

    fn update(&mut self, message: Self::Message);
}

impl<A: State<'static>> StateStatic for A {
    type Message = <A as State<'static>>::Message;

    fn update(&mut self, message: Self::Message) {
        <A as State<'static>>::update(self, message)
    }
}

pub struct Drive<M> {
    sender: UnboundedSender<M>,
    receiver: UnboundedReceiver<M>,
}

#[derive(Clone)]
pub struct DriveState<M>(UnboundedSender<M>);

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
        self.receiver.recv().await
    }

    pub async fn run(mut self, state: &mut impl State<'_, Message = M>) {
        drop(self.sender);
        while let Some(message) = self.receiver.recv().await {
            state.update(message)
        }
    }
}
