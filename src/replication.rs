use std::{
    collections::{BTreeMap, HashMap},
    net::SocketAddr,
};

use borsh::{BorshDeserialize, BorshSerialize};

use crate::{
    crypto::{Signer, Verifier},
    submit::Handle,
    task::BackgroundSpawner,
    Addr,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddrBook {
    Socket(SocketAddrBook),
    Untyped(UntypedAddrBook),
}

impl AddrBook {
    pub fn replica_addr(&self, id: u8) -> crate::Result<Addr> {
        match self {
            Self::Socket(book) => book.replica_addr(id),
            Self::Untyped(book) => Ok(book.replica_addr(id)),
        }
    }

    pub fn replica_addrs(&self) -> impl Iterator<Item = Addr> + '_ {
        // TODO make it more efficient and elegant
        match self {
            Self::Socket(book) => book.replica_addrs().collect::<Vec<_>>(),
            Self::Untyped(book) => book.replica_addrs().collect(),
        }
        .into_iter()
    }

    pub fn client_addr(&self, id: u32) -> crate::Result<Addr> {
        match self {
            Self::Socket(book) => book.client_addr(id),
            Self::Untyped(book) => Ok(book.client_addr(id)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct SocketAddrBook {
    replica_addrs: HashMap<u8, SocketAddr>,
    client_addrs: BTreeMap<u32, SocketAddr>,
}

impl SocketAddrBook {
    fn replica_addr(&self, id: u8) -> crate::Result<Addr> {
        Ok(Addr::Socket(
            *self
                .replica_addrs
                .get(&id)
                .ok_or(crate::err!("replica address not found"))?,
        ))
    }

    pub fn remove_addr(&mut self, id: u8) -> Option<SocketAddr> {
        self.replica_addrs.remove(&id)
    }

    fn replica_addrs(&self) -> impl Iterator<Item = Addr> + '_ {
        self.replica_addrs.values().copied().map(Addr::Socket)
    }

    fn client_addr(&self, id: u32) -> crate::Result<Addr> {
        Ok(Addr::Socket(
            *self
                .client_addrs
                .range(id..)
                .next()
                .ok_or(crate::err!("unexpected empty client addresses"))?
                .1,
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UntypedAddrBook {
    pub num_replica: usize,
    pub replica_id: u8,
}

impl UntypedAddrBook {
    fn replica_addr(&self, id: u8) -> Addr {
        Addr::Untyped(format!("replica-{id}"))
    }

    fn replica_addrs(&self) -> impl Iterator<Item = Addr> + '_ {
        (0..self.replica_id)
            .chain(self.replica_id + 1..self.num_replica as _)
            .map(|i| Addr::Untyped(format!("replica-{i}")))
    }

    fn client_addr(&self, id: u32) -> Addr {
        Addr::Untyped(format!("client-{id}"))
    }
}

#[derive(Debug)]
pub struct Replica {
    pub id: u8,
    pub num_replica: usize,
    pub num_faulty: usize,
    pub app: Handle<Vec<u8>, Vec<u8>>,
    pub spawner: BackgroundSpawner,
    pub signer: Signer,
    pub verifiers: HashMap<u8, Verifier>,
    pub addr_book: AddrBook,
}

#[derive(Debug)]
pub struct Client {
    pub id: u32,
    pub num_replica: usize,
    pub num_faulty: usize,
    pub spawner: BackgroundSpawner,
    pub addr_book: AddrBook,
}
