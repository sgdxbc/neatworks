use std::{any::Any, marker::PhantomData};

use tokio::{
    spawn,
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    task::JoinHandle,
};

pub trait State<'a>: Any {
    type Message;

    fn update(&mut self, message: Self::Message);
}

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

#[derive(Debug)]
pub struct Detached<A: StateStatic> {
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

impl<A: StateStatic> From<A> for Detached<A> {
    fn from(value: A) -> Self {
        Self {
            inbox: unbounded_channel(),
            actor: value,
        }
    }
}

impl<A: StateStatic> Detached<A> {
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

impl<'a, M: 'static> State<'a> for Inbox<M> {
    type Message = M;

    fn update(&mut self, message: Self::Message) {
        if self.0.send(message).is_err() {
            //
        }
    }
}

// this currently not work, because GAT is not compatible with trait object (yet)
// pub type Effect<M> = Effect2<'static, M>;
// pub type Effect2<'a, M> = Box<dyn State<Message<'a> = M>>;
pub type Effect<M> = Box<dyn State<'static, Message = M>>;

// TODO need to be more general

pub struct Adapt<F, M, A>(F, A, PhantomData<M>);

impl<'a, F, M: 'static, A> State<'_> for Adapt<F, M, A>
where
    A: State<'a>,
    F: FnMut(M) -> A::Message + 'static,
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

impl<'a, F, M, A> AdaptFn<M, A> for F
where
    A: State<'a>,
    F: FnMut(M) -> A::Message,
{
    fn then(self, actor: A) -> Adapt<Self, M, A>
    where
        Self: Sized,
    {
        Adapt(self, actor, PhantomData)
    }
}
