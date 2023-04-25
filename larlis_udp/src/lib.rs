use std::{net::SocketAddr, sync::Arc};

use larlis_core::actor;
use tokio::{net::UdpSocket, spawn};

pub struct In<A> {
    socket: Arc<UdpSocket>,
    actor: A,
}

impl<A> In<A> {
    pub fn new(socket: Arc<UdpSocket>, actor: A) -> Self {
        Self { socket, actor }
    }

    pub async fn start(&mut self)
    where
        A: for<'a> actor::State<'a, Message = (SocketAddr, &'a [u8])>,
    {
        let mut buf = vec![0; 65536];
        loop {
            let (len, remote) = self.socket.recv_from(&mut buf).await.unwrap();
            self.actor.update((remote, &buf[..len]))
        }
    }
}

pub struct Out(pub Arc<UdpSocket>);

impl actor::State<'_> for Out {
    type Message = (SocketAddr, Vec<u8>);

    fn update(&mut self, message: Self::Message) {
        let (target, buf) = message;
        let socket = self.0.clone();
        spawn(async move {
            socket.send_to(&buf, target).await.unwrap();
        });
    }
}
