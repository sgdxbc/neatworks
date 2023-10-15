use std::{net::SocketAddr, time::Duration};

use serde::Serialize;

use self::{crypto::DigestHash, ordered_multicast::OrderedMulticast};

pub mod crypto;
pub mod ordered_multicast;
pub mod simulated;
pub mod tokio;

pub type ReplicaIndex = u8;
pub type ClientIndex = u16;

#[derive(Debug)]
pub enum Context<M> {
    Tokio(tokio::Context),
    Simulated(simulated::Context<M>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Addr {
    Socket(SocketAddr),
    Simulated(simulated::Addr),
    Multicast,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum To {
    Addr(Addr),
    Addrs(Vec<Addr>),

    Replica(ReplicaIndex),
    Client(ClientIndex),
    Clients(Vec<ClientIndex>),
    AllReplica,
    Loopback,
    AllReplicaWithLoopback,
}

impl<M> Context<M> {
    pub fn addr(&self) -> Addr {
        match self {
            Self::Tokio(context) => Addr::Socket(context.source),
            Self::Simulated(context) => Addr::Simulated(context.source),
        }
    }

    pub fn num_faulty(&self) -> usize {
        match self {
            Self::Tokio(context) => context.config.num_faulty,
            Self::Simulated(context) => context.num_faulty,
        }
    }

    pub fn num_replica(&self) -> usize {
        match self {
            Self::Tokio(context) => context.config.replica_addrs.len(),
            Self::Simulated(context) => context.num_replica,
        }
    }

    pub fn send<N>(&mut self, to: To, message: N)
    where
        M: crypto::Sign<N> + Serialize + Clone,
    {
        match self {
            Self::Tokio(context) => context.send::<M, _>(to, message),
            Self::Simulated(context) => context.send(to, message),
        }
    }

    pub fn send_ordered_multicast<N>(&mut self, message: N)
    where
        N: Serialize + DigestHash,
        OrderedMulticast<N>: Into<M>,
    {
        match self {
            Self::Tokio(context) => context.send_ordered_multicast(message),
            Self::Simulated(_) => todo!(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TimerId {
    Tokio(tokio::TimerId),
    Simulated(simulated::TimerId),
}

impl<M> Context<M> {
    pub fn set(&mut self, duration: Duration) -> TimerId {
        match self {
            Self::Tokio(context) => TimerId::Tokio(context.set(duration)),
            Self::Simulated(context) => TimerId::Simulated(context.set(duration)),
        }
    }

    pub fn unset(&mut self, id: TimerId) {
        match (self, id) {
            (Self::Tokio(context), TimerId::Tokio(id)) => context.unset(id),
            (Self::Simulated(context), TimerId::Simulated(id)) => context.unset(id),
            _ => unimplemented!(),
        }
    }
}

pub trait Receivers {
    type Message;

    fn handle(&mut self, receiver: Addr, remote: Addr, message: Self::Message);

    #[allow(unused_variables)]
    fn handle_loopback(&mut self, receiver: Addr, message: Self::Message) {
        unreachable!()
    }

    fn on_timer(&mut self, receiver: Addr, id: TimerId);

    fn on_pace(&mut self) {}
}

pub trait OrderedMulticastReceivers
where
    Self: Receivers,
{
    type Message;
}
