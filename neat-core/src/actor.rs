use flume::{Receiver, Sender};

pub trait State<Message> {
    fn update(&mut self, message: Message);

    fn boxed(self) -> Box<dyn State<Message>>
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

impl<M, T: State<M> + ?Sized> State<M> for &mut T {
    fn update(&mut self, message: M) {
        T::update(self, message)
    }
}

impl<M, T: State<M> + ?Sized> State<M> for Box<T> {
    fn update(&mut self, message: M) {
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

pub struct Filtered<S>(pub S);

// TODO can (or is it necessary to) generalize `Option<_>` into `impl Into<_>`?
impl<M, S> State<Option<M>> for Filtered<S>
where
    S: State<M>,
{
    fn update(&mut self, message: Option<M>) {
        if let Some(message) = message {
            self.0.update(message)
        }
    }
}
