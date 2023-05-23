use std::marker::PhantomData;
use std::net::SocketAddr;

use crate::app::FunctionalState;

pub use crate::app::Message as App;

pub trait Lift<S, M> {
    type Out<'output>
    where
        Self: 'output,
        S: 'output;

    fn update<'a>(&'a mut self, state: &'a mut S, message: M) -> Self::Out<'a>;
}

pub use crate::dispatch::Message as Dispatch;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct DispatchLift;
// TODO

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Timeout<T> {
    Set(T),
    Reset(T),
    Unset(T),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct TimeoutLift;

impl<S, M> Lift<S, Timeout<M>> for TimeoutLift
where
    S: FunctionalState<M>,
{
    type Out<'o> = Timeout<S::Output<'o>> where Self: 'o, S: 'o;

    fn update<'a>(&'a mut self, state: &'a mut S, message: Timeout<M>) -> Self::Out<'a> {
        match message {
            Timeout::Set(message) => Timeout::Set(state.update(message)),
            Timeout::Reset(message) => Timeout::Reset(state.update(message)),
            Timeout::Unset(message) => Timeout::Unset(state.update(message)),
        }
    }
}

pub type Transport<T> = (SocketAddr, T);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct TransportLift;

impl<S, M> Lift<S, Transport<M>> for TransportLift
where
    S: FunctionalState<M>,
{
    type Out<'o> = Transport<S::Output<'o>> where Self: 'o, S: 'o;

    fn update<'a>(&'a mut self, state: &'a mut S, message: Transport<M>) -> Self::Out<'a> {
        let (addr, message) = message;
        (addr, state.update(message))
    }
}

// TODO better name
pub enum Egress<K, M> {
    To(K, M),
    ToAll(M),
}

impl<K, M> Egress<K, M> {
    pub fn to(k: K) -> impl FnOnce(M) -> Self {
        move |m| Self::To(k, m)
    }
}

// type inference works better with `K`
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct EgressLift<K>(PhantomData<K>);

impl<S, K, M> Lift<S, Egress<K, M>> for EgressLift<K>
where
    S: FunctionalState<M>,
{
    type Out<'o> = Egress<K, S::Output<'o>> where Self: 'o, S: 'o;

    fn update<'a>(&'a mut self, state: &'a mut S, message: Egress<K, M>) -> Self::Out<'a> {
        match message {
            Egress::To(dest, message) => Egress::To(dest, state.update(message)),
            Egress::ToAll(message) => Egress::ToAll(state.update(message)),
        }
    }
}
