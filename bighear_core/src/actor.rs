use std::mem::replace;

use tokio::{
    spawn,
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    task::JoinHandle,
};

pub trait ActorState {
    type Message;
    fn update(&mut self, message: Self::Message);
}

#[derive(Debug)]
pub struct Detached<A: ActorState> {
    inbox: (UnboundedSender<A::Message>, UnboundedReceiver<A::Message>),
    actor: A,
}

#[derive(Debug, Clone)]
pub struct Inbox<M>(UnboundedSender<M>);

impl<A: ActorState> From<A> for Detached<A> {
    fn from(value: A) -> Self {
        Self {
            inbox: unbounded_channel(),
            actor: value,
        }
    }
}

impl<A: ActorState> Detached<A> {
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

pub enum ActorHandle<A: ActorState> {
    Inlined(A),
    Detached(Inbox<A::Message>),
    Intermediate, // avoid e.g. option dance
}

impl<A: ActorState> ActorState for ActorHandle<A> {
    type Message = A::Message;
    fn update(&mut self, message: Self::Message) {
        match self {
            ActorHandle::Inlined(actor) => actor.update(message),
            ActorHandle::Detached(inbox) => {
                // or just trigger backward panic chain?
                if inbox.0.send(message).is_err() {
                    //
                }
            }
            ActorHandle::Intermediate => unreachable!(),
        }
    }
}

impl<A: ActorState> From<A> for ActorHandle<A> {
    fn from(value: A) -> Self {
        Self::Inlined(value)
    }
}

impl<A: ActorState> ActorHandle<A> {
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
}
