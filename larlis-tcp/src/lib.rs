use std::{collections::HashMap, net::SocketAddr};

use larlis_core::actor::{SharedClone, State};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, BufStream},
    net::{TcpListener, TcpSocket, TcpStream},
    select, spawn,
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
};

pub type Message = (SocketAddr, Vec<u8>);

#[derive(Debug)]
pub enum TransportMessage {
    Accept(SocketAddr, TcpStream),
    Send(SocketAddr, Vec<u8>),
    Close(SocketAddr),
}

impl From<(SocketAddr, Vec<u8>)> for TransportMessage {
    fn from(value: (SocketAddr, Vec<u8>)) -> Self {
        let (dest, buf) = value;
        Self::Send(dest, buf)
    }
}

// if behave as a client, i.e. never `accept` any connection, this `Transport`
// may be embedded into upstream state
// so upstream probably need to be wrapped into `Drive` to work with `Transport`
// i.e. `S` should be a `DriveState`, to avoid circular ownership (and also
// fulfill the `SharedClone` bound)
pub struct Transport<S> {
    addr: SocketAddr,
    egress: HashMap<SocketAddr, UnboundedSender<Vec<u8>>>,
    pub state: S,
}

impl<S> Transport<S> {
    pub fn bind(addr: SocketAddr, state: S) -> Self {
        Self {
            addr,
            egress: Default::default(),
            state,
        }
    }

    async fn run_connnection(
        remote: SocketAddr,
        stream: TcpStream,
        mut state: S,
        mut egress: UnboundedReceiver<Vec<u8>>,
    ) where
        S: for<'m> State<'m, Message = (SocketAddr, &'m [u8])> + Send + 'static,
    {
        assert_eq!(stream.peer_addr().unwrap(), remote);
        // i don't think we need concurrent read-write support
        // we should use this transport to support some stop-go protocols
        // and write some dedicated implementation for massive bidirectional
        // data exchange

        let mut stream = BufStream::new(stream);
        let mut buf = vec![0; 65536]; //
        loop {
            select! {
                result = stream.read_u32() => {
                    let len = result.unwrap(); // TODO
                    stream.read_exact(&mut buf[..len as _]).await.unwrap();
                    state.update((remote, &buf[..len as _]));
                }
                message = egress.recv() => {
                    let message = message.unwrap(); // TODO
                    assert!(message.len() <= u32::MAX as _);
                    stream.write_u32(message.len() as _).await.unwrap();
                    stream.write_all(&message).await.unwrap();
                }
            }
        }
    }
}

impl<S> State<'_> for Transport<S>
where
    S: SharedClone + for<'m> State<'m, Message = (SocketAddr, &'m [u8])> + Send + 'static,
{
    type Message = TransportMessage;

    fn update(&mut self, message: Self::Message) {
        match message {
            // `connect` 5-tuple, then `accept` the same 5-tuple almost at the
            // same time, i.e. both side `connect` to each other
            // assuming `stream` will be exact the same instance of the one
            // already in `run_connection` (and channeled in `egress`)
            // hope that is true (or even better this case never happen)
            TransportMessage::Accept(remote, stream) => {
                self.egress.entry(remote).or_insert_with(|| {
                    let egress = unbounded_channel();
                    spawn(Self::run_connnection(
                        remote,
                        stream,
                        self.state.clone(),
                        egress.1,
                    ));
                    egress.0
                });
            }

            TransportMessage::Send(dest, buf) => {
                let state = self.state.clone();
                let addr = self.addr;
                self.egress
                    .entry(dest)
                    .or_insert_with(move || {
                        let egress = unbounded_channel();
                        spawn(async move {
                            let socket = TcpSocket::new_v4().unwrap();
                            socket.set_reuseaddr(true).unwrap(); //
                            socket.bind(addr).unwrap();
                            Self::run_connnection(
                                dest,
                                socket.connect(dest).await.unwrap(),
                                state,
                                egress.1,
                            )
                            .await;
                        });
                        egress.0
                    })
                    .send(buf)
                    .unwrap();
            }

            TransportMessage::Close(remote) => {
                self.egress.remove(&remote); // assert exist and closed?
            }
        }
    }
}

pub struct Accept<S> {
    listener: TcpListener,
    pub state: S,
}

impl<S> Accept<S> {
    pub fn bind(addr: SocketAddr, state: S) -> Self {
        let socket = TcpSocket::new_v4().unwrap();
        socket.set_reuseaddr(true).unwrap(); //
        socket.bind(addr).unwrap();
        Self {
            listener: socket.listen(4096).unwrap(),
            state,
        }
    }

    pub async fn start(&mut self)
    where
        S: for<'m> State<'m, Message = TransportMessage>,
    {
        loop {
            let (stream, remote) = self.listener.accept().await.unwrap();
            self.state.update(TransportMessage::Accept(remote, stream));
        }
    }
}

#[derive(Debug)]
pub struct Connection<S> {
    pub stream: TcpStream,
    pub state: S,
    egress: (UnboundedSender<Vec<u8>>, UnboundedReceiver<Vec<u8>>),
}

#[derive(Debug, Clone)]
pub struct ConnectionOut(UnboundedSender<Vec<u8>>);

impl<S> Connection<S> {
    pub fn new(stream: TcpStream, state: S) -> Self {
        Self {
            stream,
            state,
            egress: unbounded_channel(),
        }
    }

    pub async fn connect(local_addr: SocketAddr, remote_addr: SocketAddr, state: S) -> Self {
        let socket = TcpSocket::new_v4().unwrap();
        socket.set_reuseaddr(true).unwrap();
        socket.bind(local_addr).unwrap();
        Self::new(socket.connect(remote_addr).await.unwrap(), state)
    }

    pub fn egress_state(&self) -> ConnectionOut {
        ConnectionOut(self.egress.0.clone())
    }

    pub async fn start(&mut self)
    where
        S: for<'m> State<'m, Message = &'m [u8]>,
    {
        let mut buf = vec![0; 65536]; //
        loop {
            select! {
                len = self.stream.read_u32() => {
                    let Ok(len) = len else {
                        //
                        return;
                    };
                    if self.stream.read_exact(&mut buf[..len as _]).await.is_err() {
                        //
                        return;
                    }
                    self.state.update(&buf[..len as _]);
                }
                message = self.egress.1.recv() => {
                    let message = message.unwrap(); //
                    if self.stream.write_u32(message.len() as _).await.is_err()
                        || self.stream.write_all(&message).await.is_err()
                        || self.stream.flush().await.is_err()
                    {
                        //
                        return;
                    }
                }
            }
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

    pub async fn accept<S>(&self, state: S) -> (SocketAddr, Connection<S>) {
        let (stream, remote) = self.0.accept().await.unwrap();
        (remote, Connection::new(stream, state))
    }
}
