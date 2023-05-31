use std::net::SocketAddr;

use neat_core::{message::Transport, State};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufStream},
    net::{TcpListener, TcpSocket, TcpStream},
    select,
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
};

// alternative design: select tx/rx in the same loop over split into two loops
// alternative design: unreliable tx over actively retry/back propogation

#[derive(Debug)]
pub struct GeneralConnection<T, S, D> {
    pub remote_addr: SocketAddr,
    pub stream: T,
    egress: (Option<UnboundedSender<Vec<u8>>>, UnboundedReceiver<Vec<u8>>),

    pub state: S,
    pub disconnected: D,
}

pub type Connection<S, D> = GeneralConnection<BufStream<TcpStream>, S, D>;

#[derive(Debug, Clone)]
pub struct ConnectionOut(UnboundedSender<Vec<u8>>);

impl<T, S, D> GeneralConnection<T, S, D> {
    pub fn new(stream: T, remote_addr: SocketAddr, state: S, disconnected: D) -> Self {
        let egress = unbounded_channel();
        Self {
            stream,
            remote_addr,
            state,
            disconnected,
            egress: (Some(egress.0), egress.1),
        }
    }

    pub fn replace_stream<U>(self, stream: U) -> (GeneralConnection<U, S, D>, T) {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Disconnected(SocketAddr);

impl<T, S, D> GeneralConnection<T, S, D> {
    pub fn out_state(&self) -> ConnectionOut {
        ConnectionOut(self.egress.0.clone().unwrap())
    }

    pub async fn start(&mut self)
    where
        // require Unpin or pin it locally?
        T: AsyncRead + AsyncWrite + Unpin,
        S: for<'m> State<Transport<&'m [u8]>>,
        D: State<Disconnected>,
    {
        // this should make sense even when `start` is called multiple times
        // revise this if actually it is not
        // also, rethink about whether we should allow `start` to be called
        // multiple times
        drop(self.egress.0.take());
        let mut buf = vec![0; 65536]; //
        let mut local_closed = false;
        loop {
            select! {
                len = self.stream.read_u32() => {
                    let Ok(len) = len else {
                        // broken connection
                        break;
                    };
                    if self.stream.read_exact(&mut buf[..len as _]).await.is_err() {
                        // broken connection
                        break;
                    }
                    self.state.update((self.remote_addr, &buf[..len as _]));
                }
                message = self.egress.1.recv(), if !local_closed => {
                    let Some(message) = message else {
                        // all message producers dropped
                        local_closed = true;
                        // do not break here because there could still be
                        // incoming messages that user is waiting for
                        // if needed, add a active closing interface (that is
                        // more graceful than drop the Connection)
                        continue;
                    };
                    if self.stream.write_u32(message.len() as _).await.is_err()
                        || self.stream.write_all(&message).await.is_err()
                        || self.stream.flush().await.is_err()
                    {
                        // broken connection
                        break;
                    }
                }
            }
        }
        self.disconnected.update(Disconnected(self.remote_addr))
    }
}

impl State<Vec<u8>> for ConnectionOut {
    fn update(&mut self, message: Vec<u8>) {
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
