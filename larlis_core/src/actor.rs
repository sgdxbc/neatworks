use std::{marker::PhantomData, mem::replace};

use tokio::{
    spawn,
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    task::JoinHandle,
};

pub trait StateCore {
    type Message<'a>;

    fn update<'a>(&mut self, message: Self::Message<'a>);
}

pub trait State: StateCore {
    type OwnedMessage;

    fn update_owned(&mut self, message: Self::OwnedMessage);

    fn adapt<'a, M, F>(self, adapter: F) -> Adapt<F, M, Self>
    where
        Self: Sized,
    {
        Adapt(adapter, self, PhantomData)
    }
}

impl<A: StateCore> State for A {
    type OwnedMessage = Self::Message<'static>;

    fn update_owned(&mut self, message: Self::OwnedMessage) {
        self.update(message)
    }
}

#[derive(Debug)]
pub struct Detached<A: State> {
    inbox: (
        UnboundedSender<A::OwnedMessage>,
        UnboundedReceiver<A::OwnedMessage>,
    ),
    actor: A,
}

#[derive(Debug)]
pub struct Inbox<M>(UnboundedSender<M>);

impl<M> Clone for Inbox<M> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<A: StateCore> From<A> for Detached<A> {
    fn from(value: A) -> Self {
        Self {
            inbox: unbounded_channel(),
            actor: value,
        }
    }
}

impl<A: State> Detached<A> {
    pub fn inbox(&self) -> Inbox<A::OwnedMessage> {
        Inbox(self.inbox.0.clone())
    }

    pub fn start(mut self) -> JoinHandle<A>
    where
        A: Send + 'static,
        A::OwnedMessage: Send,
    {
        drop(self.inbox.0);
        spawn(async move {
            while let Some(message) = self.inbox.1.recv().await {
                self.actor.update_owned(message)
            }
            self.actor
        })
    }
}

pub enum Handle<A: State> {
    Inlined(A),
    Detached(Inbox<A::OwnedMessage>),
    Intermediate, // avoid e.g. option dance
}

impl<A: StateCore> StateCore for Handle<A>
where
    for<'a> A::Message<'a>: Into<A::Message<'static>>,
{
    type Message<'a> = A::Message<'a>;
    fn update<'a>(&mut self, message: Self::Message<'a>) {
        match self {
            Handle::Inlined(actor) => actor.update(message),
            Handle::Detached(inbox) => {
                // or just trigger backward panic chain?
                if inbox.0.send(message.into()).is_err() {
                    //
                }
            }
            Handle::Intermediate => unreachable!(),
        }
    }
}

impl<A: State> Handle<A> {
    pub fn into_inner(self) -> A {
        if let Self::Inlined(actor) = self {
            actor
        } else {
            unimplemented!()
        }
    }

    pub fn detach(&mut self) -> Option<Detached<A>> {
        match self {
            Self::Inlined(_) => {
                let Self::Inlined(actor) = replace(self, Self::Intermediate) else {
                    unreachable!()
                };
                let actor = Detached::from(actor);
                *self = Self::Detached(actor.inbox());
                Some(actor)
            }
            Self::Detached(_) => None,
            Self::Intermediate => unreachable!(),
        }
    }

    pub fn try_clone(&self) -> Option<Self> {
        if let Self::Detached(inbox) = self {
            Some(Self::Detached(inbox.clone()))
        } else {
            None
        }
    }
}

impl<A: State> Clone for Handle<A> {
    fn clone(&self) -> Self {
        self.try_clone().unwrap()
    }
}

pub struct Adapt<F, M, A>(F, A, PhantomData<M>);

impl<'a, F, M, A> StateCore for Adapt<F, M, A>
where
    F: FnMut(M) -> A::Message<'a>,
    A: StateCore,
{
    type Message<'b> = M;

    fn update<'b>(&mut self, message: Self::Message<'b>) {
        self.1.update((self.0)(message))
    }
}
