use std::{collections::HashMap, net::SocketAddr};

use larlis_core::actor::State;

/// if `M` is `Ord`, then a sorted `Message<M>` will have deterministic content.
pub type Message<M> = Vec<(M, SocketAddr)>;

pub struct Service<M, E, F> {
    egress: E,
    finished: F,

    accumulated: HashMap<SocketAddr, M>,
    count: usize,
}

impl<M, E, F> Service<M, E, F> {
    pub fn new(egress: E, finished: F, count: usize) -> Self {
        Self {
            egress,
            finished,
            accumulated: Default::default(),
            count,
        }
    }
}

impl<M, E, F> State<'_> for Service<M, E, F>
where
    E: for<'m> State<'m, Message = (SocketAddr, Message<M>)>,
    F: for<'m> State<'m, Message = ()>,
    M: Clone,
{
    type Message = (SocketAddr, M);

    fn update(&mut self, message: Self::Message) {
        assert!(self.accumulated.len() < self.count);
        let (remote, message) = message;
        let prev = self.accumulated.insert(remote, message);
        assert!(prev.is_none());

        if self.accumulated.len() == self.count {
            let message = Vec::from_iter(
                self.accumulated
                    .iter()
                    .map(|(&addr, message)| (message.clone(), addr)),
            );
            for &remote in self.accumulated.keys() {
                self.egress.update((remote, message.clone()))
            }
            self.finished.update(())
        }
    }
}
