use std::net::SocketAddr;

use crate::app::PureState;

pub type Message<M> = (SocketAddr, M);

pub struct Lift<S>(pub S);

impl<'m, S> PureState<'m> for Lift<S>
where
    S: PureState<'m>,
{
    type Input = Message<S::Input>;
    type Output<'output> = Message<S::Output<'output>> where Self: 'output;

    fn update(&mut self, input: Self::Input) -> Self::Output<'_> {
        let (addr, message) = input;
        (addr, self.0.update(message))
    }
}
