use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use neat_core::{actor::SharedClone, message::Transport, State};
use tokio::{
    select, spawn,
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
};

use crate::{
    tcp::{self, Disconnected, Listener},
    tls::{Acceptor, Connector},
};

// TODO if this is verified to be working well, generalize against
// `tcp::Connection[Out]` and lift it into core crate

pub struct Control {
    // leveraging the fact that tls::ConnectionOut is just an re-export
    // consider generalize if necessary
    connections: HashMap<SocketAddr, tcp::ConnectionOut>,
    insert: (
        UnboundedSender<tcp::Connection>,
        UnboundedReceiver<tcp::Connection>,
    ),
    remove: (
        UnboundedSender<Disconnected>,
        UnboundedReceiver<Disconnected>,
    ),
    #[allow(clippy::type_complexity)] // because it is symmetric and not complex
    egress: (
        UnboundedSender<Transport<Vec<u8>>>,
        UnboundedReceiver<Transport<Vec<u8>>>,
    ),
}

pub struct ControlOut(UnboundedSender<Transport<Vec<u8>>>);

pub struct Remove(UnboundedSender<Disconnected>);

impl State<Disconnected> for Remove {
    fn update(&mut self, message: Disconnected) {
        self.0.send(message).unwrap()
    }
}

pub trait Strategy<S> {
    fn start_inbound(&mut self, connection: tcp::Connection, state: S, remove: Remove);
    fn start_outbound(&mut self, connection: tcp::Connection, state: S, remove: Remove);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Tcp;

impl<S> Strategy<S> for Tcp
where
    S: for<'m> State<Transport<&'m [u8]>> + Send + 'static,
{
    fn start_inbound(&mut self, mut connection: tcp::Connection, state: S, remove: Remove) {
        spawn(async move { connection.start(state, remove).await });
    }

    fn start_outbound(&mut self, mut connection: tcp::Connection, state: S, remove: Remove) {
        spawn(async move { connection.start(state, remove).await });
    }
}

#[derive(Clone, Default)]
pub struct Tls {
    connector: Connector,
    acceptor: Acceptor,
}

impl<S> Strategy<S> for Arc<Tls>
where
    S: for<'m> State<Transport<&'m [u8]>> + Send + 'static,
{
    fn start_inbound(&mut self, connection: tcp::Connection, state: S, remove: Remove) {
        let s = self.clone();
        spawn(async move {
            s.acceptor
                .upgrade_server(connection)
                .await
                .start(state, remove)
                .await
        });
    }

    fn start_outbound(&mut self, connection: tcp::Connection, state: S, remove: Remove) {
        let s = self.clone();
        spawn(async move {
            s.connector
                .upgrade_client(connection)
                .await
                .start(state, remove)
                .await
        });
    }
}

impl Default for Control {
    fn default() -> Self {
        Self {
            connections: Default::default(),
            insert: unbounded_channel(),
            remove: unbounded_channel(),
            egress: unbounded_channel(),
        }
    }
}

impl Control {
    pub fn start_listener(&self, listener: Listener) {
        let insert = self.insert.0.clone();
        spawn(async move {
            loop {
                let connection = listener.accept().await;
                if insert.send(connection).is_err() {
                    //
                    break;
                }
            }
        });
    }

    pub fn out_state(&self) -> ControlOut {
        ControlOut(self.egress.0.clone())
    }

    // the `start` here looks very monolithic, and seems should be decomposed
    // into several smaller units. the main two reasons not doing that:
    // 1. although it is in a similiar form, we are not looking at 3 impl of
    // `State<M>` with different `M` multiplexing together. the reason is that
    // handling these messages requires asynchronous steps, but `State::update`
    // is intended to be a synchronous interface.
    // 2. even if writing custom asynchronous functions, the channels still
    // cannot be extracted from `Control` (at least `remove`), because its
    // sender is passed into every `connection`'s working loop. since `remove`
    // has to be here, making `insert` also be here should be the least
    // confusing choice. then we don't have much to change
    //
    // if eventually this need to be revised, hope that comes soon since i am
    // planning to make this an essential facility...

    pub async fn start<T, S>(&mut self, state: T)
    where
        T: for<'m> State<Transport<&'m [u8]>> + SharedClone,
        S: Strategy<T> + Default,
    {
        let mut strategy = S::default();
        loop {
            select! {
                message = self.egress.1.recv() => {
                    let (remote, message) = message.expect("owned egress sender not dropped");
                    if !self.connections.contains_key(&remote) {
                        self.connect(remote, state.clone(), &mut strategy).await;
                    }
                    self.connections.get_mut(&remote).unwrap().update(message);
                }
                connection = self.insert.1.recv() => {
                    let connection = connection.expect("owned insert sender not dropped");
                    self
                        .connections
                        .insert(connection.remote_addr, connection.out_state());
                    strategy.start_inbound(connection, state.clone(), Remove(self.remove.0.clone()));
                }
                disconnected = self.remove.1.recv() => {
                    let Disconnected(remote) = disconnected.expect("owned remove sender not dropped");
                    // assert is some?
                    // any concurrency issue with fast disconnect/reconnect?
                    self.connections.remove(&remote);
                }
            }
        }
    }

    async fn connect<T, S>(&mut self, remote: SocketAddr, state: T, strategy: &mut S)
    where
        T: for<'m> State<Transport<&'m [u8]>>,
        S: Strategy<T>,
    {
        let connection =
            // TODO allow assign local address
            tcp::Connection::connect(SocketAddr::from(([0, 0, 0, 0], 0)), remote).await;
        self.connections
            .insert(connection.remote_addr, connection.out_state());
        strategy.start_outbound(connection, state, Remove(self.remove.0.clone()));
    }
}

impl State<Transport<Vec<u8>>> for ControlOut {
    fn update(&mut self, message: Transport<Vec<u8>>) {
        if self.0.send(message).is_err() {
            //
        }
    }
}
