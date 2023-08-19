use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use neat_core::{actor::SharedClone, message::Transport, wire::WireState, Drive, State, Wire};
use tokio::{
    select, spawn,
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
};

use crate::{
    tcp::{self, Disconnected},
    tls::{Acceptor, Connector},
};

pub struct Control {
    egress: HashMap<SocketAddr, WireState<Vec<u8>>>,
    remove: (
        UnboundedSender<Disconnected>,
        UnboundedReceiver<Disconnected>,
    ),
}

pub struct Remove(UnboundedSender<Disconnected>);

impl State<Disconnected> for Remove {
    fn update(&mut self, message: Disconnected) {
        self.0.send(message).unwrap()
    }
}

pub trait Strategy<S> {
    fn start_inbound(
        &mut self,
        connection: tcp::Connection,
        egress: Drive<Vec<u8>>,
        state: S,
        remove: Remove,
    );
    fn start_outbound(
        &mut self,
        connection: tcp::Connection,
        egress: Drive<Vec<u8>>,
        state: S,
        remove: Remove,
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Tcp;

impl<S> Strategy<S> for Tcp
where
    S: for<'m> State<Transport<&'m [u8]>> + Send + 'static,
{
    fn start_inbound(
        &mut self,
        mut connection: tcp::Connection,
        egress: Drive<Vec<u8>>,
        state: S,
        remove: Remove,
    ) {
        spawn(async move { connection.start(egress, state, remove).await });
    }

    fn start_outbound(
        &mut self,
        mut connection: tcp::Connection,
        egress: Drive<Vec<u8>>,
        state: S,
        remove: Remove,
    ) {
        spawn(async move { connection.start(egress, state, remove).await });
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
    fn start_inbound(
        &mut self,
        connection: tcp::Connection,
        egress: Drive<Vec<u8>>,
        state: S,
        remove: Remove,
    ) {
        let s = self.clone();
        spawn(async move {
            s.acceptor
                .upgrade_server(connection)
                .await
                .start(egress, state, remove)
                .await
        });
    }

    fn start_outbound(
        &mut self,
        connection: tcp::Connection,
        egress: Drive<Vec<u8>>,
        state: S,
        remove: Remove,
    ) {
        let s = self.clone();
        spawn(async move {
            s.connector
                .upgrade_client(connection)
                .await
                .start(egress, state, remove)
                .await
        });
    }
}

impl Default for Control {
    fn default() -> Self {
        Self {
            egress: Default::default(),
            remove: unbounded_channel(),
        }
    }
}

impl Control {
    pub async fn start<T, S>(
        &mut self,
        mut inbound: Drive<tcp::Connection>,
        mut egress: Drive<Transport<Vec<u8>>>,
        state: T,
    ) where
        T: for<'m> State<Transport<&'m [u8]>> + SharedClone,
        S: Strategy<T> + Default,
    {
        let mut strategy = S::default();
        loop {
            select! {
                message = egress.recv() => {
                    let (remote, message) = message.expect("egress wire not dropped");
                    if !self.egress.contains_key(&remote) {
                        self.connect(remote, state.clone(), &mut strategy).await;
                    }
                    self.egress.get_mut(&remote).unwrap().update(message);
                }
                connection = inbound.recv() => {
                    let connection = connection.expect("inbound wire not dropped");
                    let egress = Wire::default();
                    self.egress.insert(connection.remote_addr, egress.state());
                    strategy.start_inbound(
                        connection,
                        Drive::from(egress),
                        state.clone(),
                        Remove(self.remove.0.clone()),
                    );
                }
                disconnected = self.remove.1.recv() => {
                    let Disconnected(remote) = disconnected.expect("owned remove sender not dropped");
                    // assert is some?
                    // any concurrency issue with fast disconnect/reconnect?
                    self.egress.remove(&remote);
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
        let egress = Wire::default();
        self.egress.insert(connection.remote_addr, egress.state());
        strategy.start_outbound(
            connection,
            Drive::from(egress),
            state,
            Remove(self.remove.0.clone()),
        );
    }
}
