use std::{collections::HashMap, net::SocketAddr};

use larlis_core::actor::{DriveState, State};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, BufStream},
    net::{TcpListener, TcpStream},
    select, spawn,
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
};

pub type Message = (SocketAddr, Vec<u8>);

type EgressChannel = (SocketAddr, UnboundedSender<Vec<u8>>);

pub struct Transport {
    egress: HashMap<SocketAddr, UnboundedSender<Vec<u8>>>,
    state: DriveState<Message>,
    accepted: (
        UnboundedSender<EgressChannel>,
        UnboundedReceiver<EgressChannel>,
    ),
}

impl Transport {
    pub fn start_accept(&self, listener: TcpListener) {
        spawn(Self::run_accept(
            listener,
            self.state.clone(),
            self.accepted.0.clone(),
        ));
    }

    async fn run_accept(
        listener: TcpListener,
        state: DriveState<Message>,
        accepted: UnboundedSender<EgressChannel>,
    ) {
        loop {
            let (stream, remote) = listener.accept().await.unwrap();
            let egress = unbounded_channel();
            accepted.send((remote, egress.0)).unwrap();
            spawn(Self::run_connnection(
                remote,
                stream,
                state.clone(),
                egress.1,
            ));
        }
    }

    async fn run_connnection(
        remote: SocketAddr,
        stream: TcpStream,
        mut state: DriveState<Message>,
        mut egress: UnboundedReceiver<Vec<u8>>,
    ) {
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
                    state.update((remote, buf[..len as _].to_vec()));
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

impl State<'_> for Transport {
    type Message = Message;

    fn update(&mut self, message: Self::Message) {
        let (dest, buf) = message;
        self.egress
            .entry(dest)
            .or_insert_with(|| {
                let egress = unbounded_channel();
                let state = self.state.clone();
                spawn(async move {
                    let stream = TcpStream::connect(dest).await.unwrap();
                    Self::run_connnection(dest, stream, state, egress.1).await;
                });
                egress.0
            })
            .send(buf)
            .unwrap();
    }
}
