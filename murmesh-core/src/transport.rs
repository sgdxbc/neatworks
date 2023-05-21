use std::net::SocketAddr;

use crate::app::FunctionalState;

pub type Message<M> = (SocketAddr, M);

pub struct Lift<S>(pub S);

impl<'m, S> FunctionalState<'m> for Lift<S>
where
    S: FunctionalState<'m>,
{
    type Input = Message<S::Input>;
    type Output<'output> = Message<S::Output<'output>> where Self: 'output;

    fn update(&mut self, input: Self::Input) -> Self::Output<'_> {
        let (addr, message) = input;
        (addr, self.0.update(message))
    }
}
