use std::net::SocketAddr;

use crate::app::FunctionalState;

pub type Message<M> = (SocketAddr, M);

pub struct Lift<S>(pub S);

impl<M, S> FunctionalState<Message<M>> for Lift<S>
where
    S: FunctionalState<M>,
{
    type Output<'o> = Message<S::Output<'o>> where Self: 'o;

    fn update(&mut self, input: Message<M>) -> Self::Output<'_> {
        let (addr, message) = input;
        (addr, self.0.update(message))
    }
}
