use std::{net::SocketAddr, sync::Arc};

use murmesh_core::{actor, transport};
use tokio::{net::UdpSocket, spawn};

pub struct In<A> {
    pub socket: Arc<UdpSocket>,
    pub state: A,
}

impl<A> In<A> {
    pub async fn bind(addr: SocketAddr, state: A) -> Self {
        Self {
            socket: Arc::new(UdpSocket::bind(addr).await.unwrap()),
            state,
        }
    }

    pub fn new(out: &Out, state: A) -> Self {
        Self {
            socket: out.0.clone(),
            state,
        }
    }

    pub async fn start(&mut self)
    where
        A: for<'a> actor::State<'a, Message = transport::Message<&'a [u8]>>,
    {
        let mut buf = vec![0; 65536];
        loop {
            let (len, remote) = self.socket.recv_from(&mut buf).await.unwrap();
            self.state.update((remote, &buf[..len]))
        }
    }
}

#[derive(Debug, Clone)]
pub struct Out(pub Arc<UdpSocket>);

impl Out {
    pub async fn bind(addr: SocketAddr) -> Self {
        Self(Arc::new(UdpSocket::bind(addr).await.unwrap()))
    }
}

impl actor::State<'_> for Out {
    type Message = transport::Message<Vec<u8>>;

    fn update(&mut self, message: Self::Message) {
        let (target, buf) = message;
        let socket = self.0.clone();
        spawn(async move {
            socket.send_to(&buf, target).await.unwrap();
        });
    }
}
