use std::net::SocketAddr;

use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufStream},
    net::{TcpListener, TcpSocket, TcpStream},
    select,
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
};
use wm_core::actor::State;

// design choice: select tx/rx in the same loop over split into two loops
// design choice: unreliable tx over actively retry/back propogation

#[derive(Debug)]
pub struct GeneralConnection<S, D, T> {
    pub stream: T,
    pub remote_addr: SocketAddr,
    pub state: S,
    pub disconnected: D,
    egress: (UnboundedSender<Vec<u8>>, UnboundedReceiver<Vec<u8>>),
}

pub type Connection<S, D> = GeneralConnection<S, D, BufStream<TcpStream>>;

#[derive(Debug, Clone)]
pub struct ConnectionOut(UnboundedSender<Vec<u8>>);

impl<S, D, T> GeneralConnection<S, D, T> {
    pub fn new(stream: T, remote_addr: SocketAddr, state: S, disconnected: D) -> Self {
        Self {
            stream,
            remote_addr,
            state,
            disconnected,
            egress: unbounded_channel(),
        }
    }

    pub fn replace_stream<U>(self, stream: U) -> (GeneralConnection<S, D, U>, T) {
        (
            GeneralConnection {
                stream,
                remote_addr: self.remote_addr,
                state: self.state,
                disconnected: self.disconnected,
                egress: self.egress,
            },
            self.stream,
        )
    }
}

impl<S, D> Connection<S, D> {
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
        stream.set_nodelay(true).unwrap(); //
        Self::new(BufStream::new(stream), remote_addr, state, disconnected)
    }
}

impl<S, D, T> GeneralConnection<S, D, T> {
    pub fn out_state(&self) -> ConnectionOut {
        ConnectionOut(self.egress.0.clone())
    }

    pub async fn start(&mut self)
    where
        S: for<'m> State<'m, Message = (SocketAddr, &'m [u8])>,
        D: for<'m> State<'m, Message = SocketAddr>,
        // require Unpin or pin it locally?
        T: AsyncRead + AsyncWrite + Unpin,
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
        self.disconnected.update(self.remote_addr)
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
        stream.set_nodelay(true).unwrap(); //
        Connection::new(BufStream::new(stream), remote, state, disconnected)
    }
}
