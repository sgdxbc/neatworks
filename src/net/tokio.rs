use std::{
    collections::HashMap, fmt::Debug, mem::replace, net::SocketAddr, sync::Arc, time::Duration,
};

use anyhow::Context;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpSocket, TcpStream},
    sync::mpsc::{unbounded_channel, UnboundedSender},
    task::JoinSet,
};

use crate::event::{
    erased::{OnEvent, Timer},
    SendEvent, TimerId,
};

use super::{Buf, IterAddr, SendMessage};

#[derive(Debug, Clone)]
pub struct Udp(pub Arc<tokio::net::UdpSocket>);

impl Udp {
    pub async fn recv_session(
        &self,
        mut on_buf: impl FnMut(&[u8]) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        let mut buf = vec![0; 1 << 16];
        loop {
            let (len, _) = self.0.recv_from(&mut buf).await?;
            on_buf(&buf[..len])?
        }
    }
}

impl<B: Buf> SendMessage<SocketAddr, B> for Udp {
    fn send(&mut self, dest: SocketAddr, buf: B) -> anyhow::Result<()> {
        let socket = self.0.clone();
        // a broken error propagation here. no observation to the failure of `send_to`
        // by definition `SendMessage` is one-way (i.e. no complete notification) unreliable net
        // interface, so this is fine, just kindly note the fact
        // alternatively, collect sending tasks into a `JoinSet`
        // however that cannot be owned by `impl OnEvent`, which does not have a chance to poll
        // so not an ideal alternation and not conducted for now
        tokio::spawn(async move { socket.send_to(buf.as_ref(), dest).await.unwrap() });
        Ok(())
    }
}

impl<B: Buf> SendMessage<IterAddr<'_, SocketAddr>, B> for Udp {
    fn send(&mut self, dest: IterAddr<'_, SocketAddr>, buf: B) -> anyhow::Result<()> {
        for addr in dest.0 {
            self.send(addr, buf.clone())?
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct Tcp<E>(pub E);

impl<E: SendEvent<(SocketAddr, B)>, B> SendMessage<SocketAddr, B> for Tcp<E> {
    fn send(&mut self, dest: SocketAddr, message: B) -> anyhow::Result<()> {
        self.0.send((dest, message))
    }
}

impl<E: SendEvent<(SocketAddr, B)>, B: Buf> SendMessage<IterAddr<'_, SocketAddr>, B> for Tcp<E> {
    fn send(
        &mut self,
        IterAddr(addrs): IterAddr<'_, SocketAddr>,
        message: B,
    ) -> anyhow::Result<()> {
        for addr in addrs {
            self.send(addr, message.clone())?
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct TcpControl<B> {
    // TODO replace timeout-based cleanup with LRU + throttle
    connections: HashMap<SocketAddr, Connection<B>>,
}

#[derive(Debug)]
struct Connection<B> {
    sender: UnboundedSender<B>,
    in_use: bool,
    timer: TimerId,
}

impl<B> Default for TcpControl<B> {
    fn default() -> Self {
        Self {
            connections: Default::default(),
        }
    }
}

impl<B> TcpControl<B> {
    pub fn new() -> Self {
        Self::default()
    }
}

const MAX_TCP_BUF_LEN: usize = 1 << 20;

impl<B: Buf> OnEvent<(SocketAddr, B)> for TcpControl<B> {
    fn on_event(
        &mut self,
        (dest, buf): (SocketAddr, B),
        timer: &mut impl Timer<Self>,
    ) -> anyhow::Result<()> {
        if buf.as_ref().len() >= MAX_TCP_BUF_LEN {
            anyhow::bail!("TCP buf too large: {}", buf.as_ref().len())
        }

        let buf = if let Some(connection) = self.connections.get_mut(&dest) {
            connection.in_use = true;
            match UnboundedSender::send(&connection.sender, buf) {
                Ok(()) => return Ok(()),
                // fail to reuse connection, fallback to slow path
                Err(err) => err.0,
            }
        } else {
            buf
        };

        let (sender, mut receiver) = unbounded_channel::<B>();
        tokio::spawn(async move {
            // let mut stream = TcpStream::connect(dest).await.unwrap();
            let socket = TcpSocket::new_v4().unwrap();
            // socket.set_reuseaddr(true).unwrap();
            let mut stream = match socket.connect(dest).await {
                Ok(stream) => stream,
                Err(err) => panic!("{dest}: {err}"),
            };
            while let Some(buf) = receiver.recv().await {
                async {
                    stream.write_u64(buf.as_ref().len() as _).await?;
                    stream.write_all(buf.as_ref()).await?;
                    stream.flush().await?;
                    Result::<_, anyhow::Error>::Ok(())
                }
                .await
                .context(dest)
                .unwrap()
            }
        });
        sender
            .send(buf)
            .map_err(|_| anyhow::anyhow!("connection closed"))?;
        self.connections.insert(
            dest,
            Connection {
                sender,
                in_use: false,
                timer: timer.set(Duration::from_secs(10), CheckIdleConnection(dest))?,
            },
        );
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct CheckIdleConnection(SocketAddr);

impl<B> OnEvent<CheckIdleConnection> for TcpControl<B> {
    fn on_event(
        &mut self,
        CheckIdleConnection(dest): CheckIdleConnection,
        timer: &mut impl Timer<Self>,
    ) -> anyhow::Result<()> {
        let connection = self
            .connections
            .get_mut(&dest)
            .ok_or(anyhow::anyhow!("connection missing"))?;
        if !replace(&mut connection.in_use, false) {
            let connection = self.connections.remove(&dest).unwrap();
            timer.unset(connection.timer)?
        }
        Ok(())
    }
}

pub async fn tcp_listen_session(
    listener: TcpListener,
    mut on_buf: impl FnMut(&[u8]) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let mut stream_sessions = JoinSet::<anyhow::Result<_>>::new();
    let (sender, mut receiver) = unbounded_channel();
    loop {
        enum Select {
            Accept((TcpStream, SocketAddr)),
            Recv(Vec<u8>),
            JoinNext(()),
        }
        match tokio::select! {
            accept = listener.accept() => Select::Accept(accept?),
            recv = receiver.recv() => Select::Recv(recv.unwrap()),
            Some(result) = stream_sessions.join_next() => Select::JoinNext(result??),
        } {
            Select::Accept((mut stream, _)) => {
                let sender = sender.clone();
                stream_sessions.spawn(async move {
                    while let Ok(len) = stream.read_u64().await {
                        if len as usize >= MAX_TCP_BUF_LEN {
                            eprintln!(
                                "Closing connection to {:?} for too large buf: {len}",
                                stream.peer_addr()
                            );
                            break;
                        }
                        let mut buf = vec![0; len as _];
                        stream.read_exact(&mut buf).await?;
                        sender
                            .send(buf)
                            .map_err(|_| anyhow::anyhow!("channel closed"))?
                    }
                    Ok(())
                });
            }
            Select::Recv(buf) => on_buf(&buf)?,
            Select::JoinNext(()) => {}
        }
    }
}
