use std::{
    collections::{BTreeMap, HashMap},
    net::SocketAddr,
    time::Duration,
};

use borsh::{BorshDeserialize, BorshSerialize};

use crate::{
    crypto::{Signer, Verifier},
    model::{Message, Transport},
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
    pub fn replica_addr(&self, id: u8) -> Addr {
        match self {
            Self::Socket(book) => book.replica_addr(id),
            Self::Untyped(book) => book.replica_addr(id),
        }
    }

    pub fn replica_addrs(&self) -> impl Iterator<Item = Addr> {
        // TODO make it more efficient and elegant
        match self {
            Self::Socket(book) => book.replica_addrs().collect::<Vec<_>>(),
            Self::Untyped(book) => book.replica_addrs().collect(),
        }
        .into_iter()
    }

    pub fn client_addr(&self, id: u32) -> Addr {
        match self {
            Self::Socket(book) => book.client_addr(id),
            Self::Untyped(book) => book.client_addr(id),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct SocketAddrBook {
    replica_addrs: HashMap<u8, SocketAddr>,
    client_addrs: BTreeMap<u32, SocketAddr>,
}

impl SocketAddrBook {
    fn replica_addr(&self, id: u8) -> Addr {
        Addr::Socket(
            *self
                .replica_addrs
                .get(&id)
                .unwrap_or_else(|| panic!("replica {id} address not found")),
        )
    }

    pub fn remove_addr(&mut self, id: u8) -> SocketAddr {
        self.replica_addrs
            .remove(&id)
            .unwrap_or_else(|| panic!("replica {id} address not found"))
    }

    // TODO cache addresses and remove lifetime bound?
    fn replica_addrs(&self) -> impl Iterator<Item = Addr> + '_ {
        self.replica_addrs.values().copied().map(Addr::Socket)
    }

    fn client_addr(&self, id: u32) -> Addr {
        Addr::Socket(
            *self
                .client_addrs
                .range(id..)
                .next()
                .unwrap_or_else(|| panic!("unexpected empty client address"))
                .1,
        )
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

    fn replica_addrs(&self) -> impl Iterator<Item = Addr> {
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

impl Replica {
    pub fn send_to_replica<M>(&self, id: u8, message: M, transport: impl Transport<M>)
    where
        M: Message,
    {
        assert_ne!(id, self.id);
        let destination = self.addr_book.replica_addr(id);
        self.spawner
            .spawn(async move { transport.send_to(destination, message).await });
    }

    pub fn send_to_all_replica<M>(&self, message: M, transport: impl Transport<M>)
    where
        M: Message,
    {
        let destinations = self.addr_book.replica_addrs();
        self.spawner
            .spawn(async move { transport.send_to_all(destinations, message).await });
    }

    pub fn send_to_client<M>(&self, id: u32, message: M, transport: impl Transport<M>)
    where
        M: Message,
    {
        let destination = self.addr_book.client_addr(id);
        self.spawner
            .spawn(async move { transport.send_to(destination, message).await });
    }
}

#[derive(Debug)]
pub struct Client {
    pub id: u32,
    pub num_replica: usize,
    pub num_faulty: usize,
    pub spawner: BackgroundSpawner,
    pub addr_book: AddrBook,
    pub retry_interval: Duration,
}
