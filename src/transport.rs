use std::net::SocketAddr;

use futures_util::{Sink, Stream};
use tokio_stream::wrappers::UnboundedReceiverStream;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transmit<M> {
    ToAddr(SocketAddr, M),
    ToReplica(u8, M),
    ToAllReplica(M),
    ToClient(u32, M),
}

pub trait Transport<M>
where
    Self: Sink<Transmit<M>> + Unpin + Send + Clone + 'static,
{
}

pub trait Source<M>
where
    Self: Stream<Item = M> + Unpin + Send + 'static,
{
}

impl<M> Source<M> for UnboundedReceiverStream<M> where M: Send + 'static {}
