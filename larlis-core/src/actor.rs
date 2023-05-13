use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

pub trait State<'message> {
    type Message;

    fn update(&mut self, message: Self::Message);

    fn boxed(self) -> Box<dyn State<'message, Message = Self::Message>>
    where
        Self: Sized + 'static,
    {
        Box::new(self)
    }

    fn filtered(self) -> Filtered<Self>
    where
        Self: Sized,
    {
        Filtered(self)
    }
}

impl<'m, T: State<'m>> State<'m> for &mut T {
    type Message = T::Message;

    fn update(&mut self, message: Self::Message) {
        T::update(self, message)
    }
}

impl<'m, T: State<'m>> State<'m> for Box<T> {
    type Message = T::Message;

    fn update(&mut self, message: Self::Message) {
        T::update(self, message)
    }
}

pub trait SharedClone: Clone {}

impl<T: SharedClone> SharedClone for &T {}

impl<T: SharedClone> SharedClone for Box<T> {}

impl<T> SharedClone for std::rc::Rc<T> {}

impl<T> SharedClone for std::sync::Arc<T> {}

#[derive(Debug)]
pub struct Wire<M> {
    sender: UnboundedSender<M>,
    receiver: UnboundedReceiver<M>,
}

pub struct WireState<M>(UnboundedSender<M>);

impl<M> Clone for WireState<M> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<M> SharedClone for WireState<M> {}

impl<M> State<'_> for WireState<M> {
    type Message = M;

    fn update(&mut self, message: Self::Message) {
        if self.0.send(message).is_err() {
            //
        }
    }
}

impl<M> Default for Wire<M> {
    fn default() -> Self {
        let (sender, receiver) = unbounded_channel();
        Self { sender, receiver }
    }
}

#[derive(Debug)]
pub struct Drive<M>(UnboundedReceiver<M>);

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
        self.0.recv().await
    }

    pub async fn run(&mut self, mut state: impl State<'_, Message = M>) {
        while let Some(message) = self.0.recv().await {
            state.update(message)
        }
    }
}

pub struct Filtered<S>(pub S);

impl<'m, S> State<'m> for Filtered<S>
where
    S: State<'m>,
{
    type Message = Option<S::Message>; // TODO can generalize `Option<_>` into `impl Into<_>`?

    fn update(&mut self, message: Self::Message) {
        if let Some(message) = message {
            self.0.update(message)
        }
    }
}
