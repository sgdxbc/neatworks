use std::{net::SocketAddr, sync::Arc};

use larlis_core::ActorState;
use tokio::net::UdpSocket;

pub struct In<A> {
    socket: Arc<UdpSocket>,
    actor: A,
}

impl<A> In<A> {
    pub fn new(socket: Arc<UdpSocket>, actor: A) -> Self {
        Self { socket, actor }
    }

    pub async fn start_bytes(&mut self)
    where
        A: for<'a> ActorState<Message<'a> = (SocketAddr, &'a [u8])>,
    {
        let mut buf = vec![0; 65536];
        loop {
            let (len, remote) = self.socket.recv_from(&mut buf).await.unwrap();
            self.actor.update((remote, &buf[..len]))
        }
    }
}

pub struct Out(Arc<UdpSocket>);

impl Out {
    pub async fn new(socket: Arc<UdpSocket>) -> Self {
        socket.writable().await.unwrap();
        Self(socket)
    }
}

impl ActorState for Out {
    type Message<'a> = (SocketAddr, &'a [u8]);

    fn update<'a>(&mut self, message: Self::Message<'a>) {
        let (target, buf) = message;
        self.0.try_send_to(buf, target).unwrap();
    }
}
