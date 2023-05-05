use std::{collections::HashMap, net::SocketAddr};

use larlis_bincode::{de, ser};
use larlis_core::{
    actor::{Drive, State},
    app::{Closure, PureState},
    Dispatch,
};
use serde::{de::DeserializeOwned, Serialize};
use tokio::spawn;

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

pub async fn use_barrier<M>(addr: SocketAddr, service: SocketAddr, payload: M) -> Message<M>
where
    M: Serialize + DeserializeOwned + Send + 'static,
{
    let mut message = Drive::default();
    let mut connection = larlis_tcp::Connection::connect(
        addr,
        service,
        de().install(Closure::from(|(_, message)| message).install(message.state())),
        Drive::default().state(),
    )
    .await;
    let mut dispatch = Dispatch::default();
    dispatch.insert_state(connection.remote_addr, connection.egress_state());
    let connection = spawn(async move { connection.start().await });

    ser().install(dispatch).update((service, payload));

    let message = message.recv().await.unwrap();
    connection.abort();
    message
}

pub async fn provide_barrier<M>(addr: SocketAddr, count: usize)
where
    M: Clone + Serialize + DeserializeOwned + Send + 'static,
{
    let app_drive = Drive::default();
    let mut finished = Drive::default();
    let disconnected = Drive::default();

    let listener = larlis_tcp::Listener::bind(addr);
    let mut dispatch = Dispatch::default();
    let mut connections = Vec::new();
    for _ in 0..count {
        let mut connection = listener
            .accept(de::<M>().install(app_drive.state()), disconnected.state())
            .await;
        dispatch.insert_state(connection.remote_addr, connection.egress_state());
        connections.push(spawn(async move { connection.start().await }));
    }
    let app = Service::new(ser().install(dispatch), finished.state(), count);

    let app = spawn(async move { app_drive.run(app).await });
    finished.recv().await.unwrap();
    for connection in connections {
        connection.await.unwrap()
    }
    app.await.unwrap()
}
