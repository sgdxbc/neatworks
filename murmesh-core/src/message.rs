use std::net::SocketAddr;

use crate::app::FunctionalState;

pub use crate::app::Message as App;
pub use crate::dispatch::Message as Dispatch;

pub trait Lift<'m, M, S> {
    type Out<'output>
    where
        S: 'output;

    // borrowing `state` to give returned message a lifetime
    // is this good?
    fn update(self, state: &mut S) -> Self::Out<'_>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Timeout<T> {
    Set(T),
    Reset(T),
    Unset(T),
}

impl<'m, M, S> Lift<'m, M, S> for Timeout<M>
where
    S: FunctionalState<'m, Input = M>,
{
    type Out<'o> = Timeout<S::Output<'o>> where S: 'o;

    fn update(self, state: &mut S) -> Self::Out<'_> {
        match self {
            Self::Set(message) => Timeout::Set(state.update(message)),
            Self::Reset(message) => Timeout::Reset(state.update(message)),
            Self::Unset(message) => Timeout::Unset(state.update(message)),
        }
    }
}

pub type Transport<T> = (SocketAddr, T);

impl<'m, M, S> Lift<'m, M, S> for Transport<M>
where
    S: FunctionalState<'m, Input = M>,
{
    type Out<'o> = Transport<S::Output<'o>> where S: 'o;

    fn update(self, state: &mut S) -> Self::Out<'_> {
        let (addr, message) = self;
        (addr, state.update(message))
    }
}

// TODO better name
pub enum Egress<K, T> {
    To(K, T),
    ToAll(T),
}

impl<'m, K, M, S> Lift<'m, M, S> for Egress<K, M>
where
    S: FunctionalState<'m, Input = M>,
{
    type Out<'o> = Egress<K, S::Output<'o>> where S: 'o;

    fn update(self, state: &mut S) -> Self::Out<'_> {
        match self {
            Self::To(dest, message) => Egress::To(dest, state.update(message)),
            Self::ToAll(message) => Egress::ToAll(state.update(message)),
        }
    }
}
