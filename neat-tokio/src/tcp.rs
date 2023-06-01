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
//
// currently there's no way to actively close (the local half of) a connection
// except drop the `GeneralConnection` instance (probably after aborting the
// `GeneralConnection::start` calling task)
// this should not be a problem, since `TcpStream` does not provide any closing
// interface so there's nothing much i can do anyway

#[derive(Debug)]
pub struct GeneralConnection<T> {
    pub remote_addr: SocketAddr,
    pub stream: T,
    egress: (UnboundedSender<Vec<u8>>, UnboundedReceiver<Vec<u8>>),
}

pub type Connection = GeneralConnection<BufStream<TcpStream>>;

#[derive(Debug, Clone)]
pub struct ConnectionOut(UnboundedSender<Vec<u8>>);

impl<T> GeneralConnection<T> {
    pub fn new(stream: T, remote_addr: SocketAddr) -> Self {
        Self {
            stream,
            remote_addr,
            egress: unbounded_channel(),
        }
    }

    pub fn replace_stream<U>(self, stream: U) -> (GeneralConnection<U>, T) {
        (
            GeneralConnection {
                stream,
                remote_addr: self.remote_addr,
                egress: self.egress,
            },
            self.stream,
        )
    }
}

impl Connection {
    pub async fn connect(local_addr: SocketAddr, remote_addr: SocketAddr) -> Self {
        let socket = TcpSocket::new_v4().unwrap();
        socket.set_reuseaddr(true).unwrap();
        socket.bind(local_addr).unwrap();
        let stream = socket.connect(remote_addr).await.unwrap();
        stream.set_nodelay(true).unwrap(); //
        Self::new(BufStream::new(stream), remote_addr)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Disconnected(pub SocketAddr);

impl<T> GeneralConnection<T> {
    pub fn out_state(&self) -> ConnectionOut {
        ConnectionOut(self.egress.0.clone())
    }

    pub async fn start(
        &mut self,
        mut state: impl for<'m> State<Transport<&'m [u8]>>,
        mut disconnected: impl State<Disconnected>,
    ) where
        // require Unpin or pin it locally?
        T: AsyncRead + AsyncWrite + Unpin,
    {
        let mut buf = vec![0; 65536]; //
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
                    state.update((self.remote_addr, &buf[..len as _]));
                }
                message = self.egress.1.recv() => {
                    let message = message.expect("owned egress sender not dropped");
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
        disconnected.update(Disconnected(self.remote_addr))
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

    pub async fn accept(&self) -> Connection {
        let (stream, remote) = self.0.accept().await.unwrap();
        stream.set_nodelay(true).unwrap(); //
        Connection::new(BufStream::new(stream), remote)
    }
}
