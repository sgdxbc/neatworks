use std::time::Duration;

use serde::Serialize;

use self::{crypto::DigestHash, ordered_multicast::OrderedMulticast};

pub mod crypto;
pub mod ordered_multicast;
pub mod tokio;

pub type ReplicaIndex = u8;
pub type ClientIndex = u16;

#[derive(Debug)]
pub enum Context<M> {
    Tokio(tokio::Context),
    Phantom(std::marker::PhantomData<M>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Host {
    Client(ClientIndex),
    Replica(ReplicaIndex),
    Multicast,             // as receiver
    UnkownMulticastSource, // as remote
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum To {
    Host(Host),
    Hosts(Vec<Host>),
    AllReplica,
    Loopback,
    AllReplicaWithLoopback,
}

impl To {
    pub fn replica(index: ReplicaIndex) -> Self {
        Self::Host(Host::Replica(index))
    }

    pub fn client(index: ClientIndex) -> Self {
        Self::Host(Host::Client(index))
    }
}

impl<M> Context<M> {
    pub fn num_faulty(&self) -> usize {
        match self {
            Self::Tokio(context) => context.num_faulty(),
            _ => unimplemented!(),
        }
    }

    pub fn num_replica(&self) -> usize {
        match self {
            Self::Tokio(context) => context.num_replica(),
            _ => unimplemented!(),
        }
    }

    pub fn send<N>(&mut self, to: To, message: N)
    where
        M: crypto::Sign<N> + Serialize,
    {
        match self {
            Self::Tokio(context) => context.send::<M, _>(to, message),
            _ => unimplemented!(),
        }
    }

    pub fn send_ordered_multicast<N>(&mut self, message: N)
    where
        N: Serialize + DigestHash,
        OrderedMulticast<N>: Into<M>,
    {
        match self {
            Self::Tokio(context) => context.send_ordered_multicast(message),
            _ => unimplemented!(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TimerId {
    Tokio(tokio::TimerId),
}

impl<M> Context<M> {
    pub fn set(&mut self, duration: Duration) -> TimerId {
        match self {
            Self::Tokio(context) => TimerId::Tokio(context.set(duration)),
            _ => unimplemented!(),
        }
    }

    pub fn unset(&mut self, id: TimerId) {
        match (self, id) {
            (Self::Tokio(context), TimerId::Tokio(id)) => context.unset(id),
            _ => unimplemented!(),
        }
    }
}

pub trait Receivers {
    type Message;

    fn handle(&mut self, receiver: Host, remote: Host, message: Self::Message);

    #[allow(unused_variables)]
    fn handle_loopback(&mut self, receiver: Host, message: Self::Message) {
        unreachable!()
    }

    fn on_timer(&mut self, receiver: Host, id: TimerId);

    fn on_pace(&mut self) {}
}

pub trait OrderedMulticastReceivers
where
    Self: Receivers,
{
    type Message;
}
