use serde::{Deserialize, Serialize};

use crate::crypto::Sign;

pub mod ordered_multicast;
pub mod replication;
pub mod simulated;
pub mod tokio;

#[derive(Debug)]
pub enum Context<M> {
    Tokio(tokio::Context<M>),
    Simulated(simulated::Context<M>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Addr {
    Socket(std::net::SocketAddr),
    Simulated(simulated::Addr),
    Multicast,
    Upcall,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum To {
    Addr(Addr),
    Addrs(Vec<Addr>),
    Loopback,
    AddrsWithLoopback(Vec<Addr>),
}

impl<M> Context<M> {
    pub fn addr(&self) -> Addr {
        match self {
            Self::Tokio(context) => Addr::Socket(context.source),
            Self::Simulated(context) => Addr::Simulated(context.source),
        }
    }

    pub fn send<N>(&mut self, to: To, message: N)
    where
        M: Sign<N> + Clone,
    {
        match self {
            Self::Tokio(context) => context.send(to, message),
            Self::Simulated(context) => context.send(to, message),
        }
    }

    pub fn send_buf(&self, addr: Addr, buf: impl AsRef<[u8]> + Send + Sync + 'static) {
        match self {
            Self::Tokio(context) => context.send_buf(addr, buf),
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
    pub fn set(&mut self, duration: std::time::Duration) -> TimerId {
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

pub trait MultiplexReceive {
    type Message;

    fn handle(&mut self, receiver: Addr, remote: Addr, message: Self::Message);

    #[allow(unused_variables)]
    fn handle_loopback(&mut self, receiver: Addr, message: Self::Message) {
        unreachable!()
    }

    fn on_timer(&mut self, receiver: Addr, id: TimerId);

    fn on_pace(&mut self) {}
}

pub trait OrderedMulticastReceive
where
    Self: MultiplexReceive,
{
    type Message;
}
