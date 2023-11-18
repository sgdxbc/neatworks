use std::sync::Arc;

use borsh::{BorshDeserialize, BorshSerialize};
use derive_more::From;
use tokio::sync::mpsc::{self, UnboundedSender};

use crate::model::{EventSource, Message, Transport};

#[derive(Debug, Clone, From)]
pub struct UdpSocket(Arc<tokio::net::UdpSocket>);

impl UdpSocket {
    pub async fn listen_loop<M, E>(&self, event_sender: UnboundedSender<E>) -> crate::Result<()>
    where
        M: BorshDeserialize + Into<E> + Send + 'static,
    {
        let mut buf = vec![0; 65536];
        loop {
            let (len, _remote) = self.0.recv_from(&mut buf).await?;
            event_sender
                .send(borsh::from_slice::<M>(&buf[..len])?.into())
                .map_err(|_| crate::err!("unexpected event channel closing"))?
        }
    }
}

#[async_trait::async_trait]
impl<M, E> EventSource<E> for mpsc::UnboundedReceiver<M>
where
    M: Into<E> + Send,
{
    async fn next(&mut self) -> Option<E> {
        self.recv().await.map(Into::into)
    }
}

#[derive(Debug, Clone)]
pub struct UdpTransport<M>(Arc<tokio::net::UdpSocket>, std::marker::PhantomData<M>);

impl<M> From<UdpSocket> for UdpTransport<M> {
    fn from(UdpSocket(socket): UdpSocket) -> Self {
        Self(socket, Default::default())
    }
}

#[async_trait::async_trait]
impl<M, N> Transport<M> for UdpTransport<N>
where
    M: Into<N> + Send + 'static,
    N: BorshSerialize + Send + Sync,
{
    fn addr(&self) -> crate::Addr {
        crate::Addr::Socket(self.0.local_addr().expect("retrievable local address"))
    }

    async fn send_to(&mut self, destination: crate::Addr, message: M) -> crate::Result<()>
    where
        M: Message,
    {
        let crate::Addr::Socket(destination) = destination else {
            crate::bail!("unsupported destination kind {destination:?}")
        };
        let buf = borsh::to_vec(&message.into())?;
        self.0.send_to(&buf, destination).await?;
        Ok(())
    }
}
