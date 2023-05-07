use std::net::SocketAddr;

use larlis_core::actor::State;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpSocket, TcpStream},
    select,
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
};

// design choice: select tx/rx in the same loop over split into two loops
// design choice: unreliable tx over actively retry/back propogation

#[derive(Debug)]
pub struct Connection<S, D> {
    pub stream: TcpStream,
    pub remote_addr: SocketAddr,
    pub state: S,
    pub disconnected: D,
    egress: (UnboundedSender<Vec<u8>>, UnboundedReceiver<Vec<u8>>),
}

#[derive(Debug, Clone)]
pub struct ConnectionOut(UnboundedSender<Vec<u8>>);

impl<S, D> Connection<S, D> {
    pub fn new(stream: TcpStream, remote_addr: SocketAddr, state: S, disconnected: D) -> Self {
        Self {
            stream,
            remote_addr,
            state,
            disconnected,
            egress: unbounded_channel(),
        }
    }

    pub async fn connect(
        local_addr: SocketAddr,
        remote_addr: SocketAddr,
        state: S,
        disconnected: D,
    ) -> Self {
        let socket = TcpSocket::new_v4().unwrap();
        socket.set_reuseaddr(true).unwrap();
        socket.bind(local_addr).unwrap();
        let stream = socket.connect(remote_addr).await.unwrap();
        // stream.set_nodelay(true).unwrap(); //
        Self::new(stream, remote_addr, state, disconnected)
    }

    pub fn out_state(&self) -> ConnectionOut {
        ConnectionOut(self.egress.0.clone())
    }

    pub async fn start(&mut self)
    where
        S: for<'m> State<'m, Message = (SocketAddr, &'m [u8])>,
        D: for<'m> State<'m, Message = ()>,
    {
        let mut buf = vec![0; 65536]; //
        loop {
            select! {
                len = self.stream.read_u32() => {
                    let Ok(len) = len else {
                        //
                        break;
                    };
                    if self.stream.read_exact(&mut buf[..len as _]).await.is_err() {
                        //
                        break;
                    }
                    self.state.update((self.remote_addr, &buf[..len as _]));
                }
                message = self.egress.1.recv() => {
                    let message = message.unwrap(); //
                    if self.stream.write_u32(message.len() as _).await.is_err()
                        || self.stream.write_all(&message).await.is_err()
                        || self.stream.flush().await.is_err()
                    {
                        //
                        break;
                    }
                }
            }
        }
        self.disconnected.update(())
    }
}

impl State<'_> for ConnectionOut {
    type Message = Vec<u8>;

    fn update(&mut self, message: Self::Message) {
        if self.0.send(message).is_err() {
            //
        }
    }
}

#[derive(Debug)]
pub struct Listener(pub TcpListener);

impl Listener {
    pub fn bind(addr: SocketAddr) -> Self {
        let socket = TcpSocket::new_v4().unwrap();
        socket.set_reuseaddr(true).unwrap();
        socket.bind(addr).unwrap();
        Self(socket.listen(4096).unwrap())
    }

    pub async fn accept<S, D>(&self, state: S, disconnected: D) -> Connection<S, D> {
        let (stream, remote) = self.0.accept().await.unwrap();
        Connection::new(stream, remote, state, disconnected)
    }
}
