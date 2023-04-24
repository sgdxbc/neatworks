use std::marker::PhantomData;

use tokio::{
    spawn,
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    task::JoinHandle,
};

pub trait State {
    type Message<'a>;

    fn update<'a>(&mut self, message: Self::Message<'a>);
}

// need a better name
pub trait StateMove {
    type Message;

    fn update(&mut self, message: Self::Message);
}

impl<A: StateMove> State for A {
    type Message<'a> = <Self as StateMove>::Message;

    fn update(&mut self, message: Self::Message<'_>) {
        StateMove::update(self, message)
    }
}

#[derive(Debug)]
pub struct Detached<A: StateMove> {
    inbox: (UnboundedSender<A::Message>, UnboundedReceiver<A::Message>),
    actor: A,
}

#[derive(Debug)]
pub struct Inbox<M>(UnboundedSender<M>);

impl<M> Clone for Inbox<M> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<A: StateMove> From<A> for Detached<A> {
    fn from(value: A) -> Self {
        Self {
            inbox: unbounded_channel(),
            actor: value,
        }
    }
}

impl<A: StateMove> Detached<A> {
    pub fn inbox(&self) -> Inbox<A::Message> {
        Inbox(self.inbox.0.clone())
    }

    pub fn start(mut self) -> JoinHandle<A>
    where
        A: Send + 'static,
        A::Message: Send,
    {
        drop(self.inbox.0);
        spawn(async move {
            while let Some(message) = self.inbox.1.recv().await {
                self.actor.update(message)
            }
            self.actor
        })
    }
}

impl<M> StateMove for Inbox<M> {
    type Message = M;

    fn update(&mut self, message: Self::Message) {
        if self.0.send(message).is_err() {
            //
        }
    }
}

pub type Effect<M> = Box<dyn StateMove<Message = M>>;

pub struct Adapt<F, M, A>(F, A, PhantomData<M>);

impl<'a, F, M, A> StateMove for Adapt<F, M, A>
where
    A: State,
    // M could be `&'a _`
    F: FnMut(M) -> A::Message<'a>,
{
    type Message = M;

    fn update(&mut self, message: Self::Message) {
        self.1.update((self.0)(message))
    }
}

pub trait AdaptFn<M, A> {
    fn then(self, actor: A) -> Adapt<Self, M, A>
    where
        Self: Sized;
}

impl<F, M, A> AdaptFn<M, A> for F
where
    F: FnMut(M) -> A::Message,
    A: StateMove,
{
    fn then(self, actor: A) -> Adapt<Self, M, A>
    where
        Self: Sized,
    {
        Adapt(self, actor, PhantomData)
    }
}
