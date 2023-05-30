use std::{net::SocketAddr, sync::Arc};

use neat_core::{actor, message::Transport, State};
use tokio::{net::UdpSocket, spawn};

// any better name?
#[derive(Debug, Clone)]
pub struct Socket(pub Arc<UdpSocket>);

impl Socket {
    pub async fn bind(addr: SocketAddr) -> Self {
        Self(Arc::new(UdpSocket::bind(addr).await.unwrap()))
    }

    pub async fn start(&self, mut state: impl for<'a> State<Transport<&'a [u8]>>) {
        let mut buf = vec![0; 65536];
        loop {
            let (len, remote) = self.0.recv_from(&mut buf).await.unwrap();
            state.update((remote, &buf[..len]))
        }
    }
}

impl actor::State<Transport<Vec<u8>>> for Socket {
    fn update(&mut self, message: Transport<Vec<u8>>) {
        let (target, buf) = message;
        let socket = self.0.clone();
        spawn(async move {
            socket.send_to(&buf, target).await.unwrap();
        });
    }
}
