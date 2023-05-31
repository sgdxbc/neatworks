use std::{collections::HashMap, net::SocketAddr};

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

#[async_trait::async_trait]
pub trait Strategy<S> {
    async fn on_inbound(&mut self, control: &mut Control, connection: tcp::Connection, state: S)
    where
        Self: Sized;

    async fn on_outbound(&mut self, control: &mut Control, connection: tcp::Connection, state: S)
    where
        Self: Sized;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Tcp;

#[async_trait::async_trait]
impl<S> Strategy<S> for Tcp
where
    S: for<'m> State<Transport<&'m [u8]>> + Send + 'static,
{
    async fn on_inbound(&mut self, control: &mut Control, mut connection: tcp::Connection, state: S)
    where
        Self: Sized,
    {
        control
            .connections
            .insert(connection.remote_addr, connection.out_state());
        let remove = Remove(control.remove.0.clone());
        spawn(async move { connection.start(state, remove).await });
    }

    async fn on_outbound(
        &mut self,
        control: &mut Control,
        mut connection: tcp::Connection,
        state: S,
    ) where
        Self: Sized,
    {
        control
            .connections
            .insert(connection.remote_addr, connection.out_state());
        let remove = Remove(control.remove.0.clone());
        spawn(async move { connection.start(state, remove).await });
    }
}

#[derive(Clone, Default)]
pub struct Tls {
    connector: Connector,
    acceptor: Acceptor,
}

#[async_trait::async_trait]
impl<S> Strategy<S> for Tls
where
    S: for<'m> State<Transport<&'m [u8]>> + Send + 'static,
{
    async fn on_inbound(&mut self, control: &mut Control, connection: tcp::Connection, state: S)
    where
        Self: Sized,
    {
        let mut connection = self.acceptor.upgrade_server(connection).await;
        control
            .connections
            .insert(connection.remote_addr, connection.out_state());
        let remove = Remove(control.remove.0.clone());
        spawn(async move { connection.start(state, remove).await });
    }

    async fn on_outbound(&mut self, control: &mut Control, connection: tcp::Connection, state: S)
    where
        Self: Sized,
    {
        let mut connection = self.connector.upgrade_client(connection).await;
        control
            .connections
            .insert(connection.remote_addr, connection.out_state());
        let remove = Remove(control.remove.0.clone());
        spawn(async move { connection.start(state, remove).await });
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
                    strategy.on_inbound(self, connection, state.clone()).await;
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
        strategy.on_outbound(self, connection, state).await;
    }
}

impl State<Transport<Vec<u8>>> for ControlOut {
    fn update(&mut self, message: Transport<Vec<u8>>) {
        if self.0.send(message).is_err() {
            //
        }
    }
}
