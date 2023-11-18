use std::{future::Future, sync::Arc};

use borsh::{BorshDeserialize, BorshSerialize};
use derive_more::From;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio_util::sync::CancellationToken;

use crate::model::{EventSource, Transport};

#[derive(Debug, Clone)]
pub struct BackgroundSpawner {
    err_sender: UnboundedSender<crate::Error>,
    token: CancellationToken,
}

impl BackgroundSpawner {
    pub fn spawn(&self, task: impl Future<Output = crate::Result<()>> + Send + 'static) {
        let Self { err_sender, token } = self.clone();
        tokio::spawn(async move {
            let result = tokio::select! {
                result = task => result,
                _ = token.cancelled() => return,
            };
            if let Err(err) = result {
                err_sender
                    .send(err)
                    .expect("background monitor not shutdown")
            }
        });
    }
}

#[derive(Debug)]
pub struct BackgroundMonitor(UnboundedReceiver<crate::Error>);

impl BackgroundMonitor {
    pub async fn wait(&mut self) -> crate::Result<()> {
        self.0.recv().await.map(Err).unwrap_or(Ok(()))
    }
}

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
    async fn send_to(&mut self, destination: crate::Addr, message: M) -> crate::Result<()> {
        let crate::Addr::Socket(destination) = destination else {
            crate::bail!("unsupported destination kind {destination:?}")
        };
        let buf = borsh::to_vec(&message.into())?;
        self.0.send_to(&buf, destination).await?;
        Ok(())
    }
}
