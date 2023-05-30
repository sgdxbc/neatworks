use std::{
    collections::{BTreeMap, HashMap},
    net::{IpAddr, SocketAddr},
};

use rand::Rng;

use crate::{message::Transport, State};

pub enum Message<K, M> {
    To(K, M),
    ToAll(M),
}

#[derive(Default, Clone, PartialEq, Eq)]
pub struct ClientTable {
    hosts: BTreeMap<usize, IpAddr>,
    identities: Vec<u32>,
    routes: HashMap<u32, SocketAddr>,
}

impl ClientTable {
    pub fn add_host(&mut self, host: IpAddr, count: usize, mut rng: impl Rng) {
        self.hosts.insert(self.routes.len(), host);
        for port in (50000..).take(count) {
            let identity = rng.gen();
            self.identities.push(identity);
            self.routes.insert(identity, SocketAddr::from((host, port)));
        }
    }

    pub fn addr(&self, index: usize) -> SocketAddr {
        assert!(index < 10000);
        let (&base_index, &host) = self.hosts.range(..=index).last().unwrap();
        SocketAddr::from((host, 50000 + (index - base_index) as u16))
    }

    pub fn identity(&self, index: usize) -> u32 {
        self.identities[index]
    }

    pub fn lookup_addr(&self, id: u32) -> SocketAddr {
        self.routes[&id]
    }

    pub fn len(&self) -> usize {
        self.identities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.identities.is_empty()
    }
}

// consider make a Message<u32, M> -> Transport<M> adapter for ClientTable as well
// may need more think on this since ToAll make no sense clearly

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReplicaTable {
    identities: Vec<[u8; 32]>,
    routes: Vec<IpAddr>,
}

impl ReplicaTable {
    pub fn len(&self) -> usize {
        self.identities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.identities.is_empty()
    }

    pub fn add(&mut self, host: IpAddr, mut rng: impl Rng) {
        let index = self.len();
        self.identities.push(rng.gen());
        self.routes.insert(index as _, host);
    }

    pub fn identity(&self, id: u8) -> [u8; 32] {
        self.identities[id as usize]
    }

    pub fn public_addr(&self, id: u8) -> SocketAddr {
        SocketAddr::new(self.routes[id as usize], 60001)
    }

    pub fn internal_addr(&self, id: u8) -> SocketAddr {
        SocketAddr::new(self.routes[id as usize], 60002)
    }
}

pub struct External<S>(pub ReplicaTable, pub S);

impl<S, M> State<Message<u8, M>> for External<S>
where
    S: State<Transport<M>>,
    M: Clone,
{
    fn update(&mut self, message: Message<u8, M>) {
        match message {
            Message::To(id, message) => self.1.update((self.0.public_addr(id), message)),
            Message::ToAll(message) => {
                for id in 0..self.0.len() as u8 {
                    self.1.update((self.0.public_addr(id), message.clone()))
                }
            }
        }
    }
}

pub struct Internal<S>(pub ReplicaTable, pub u8, pub S);

impl<S, M> State<Message<u8, M>> for Internal<S>
where
    S: State<Transport<M>>,
    M: Clone,
{
    fn update(&mut self, message: Message<u8, M>) {
        match message {
            Message::To(id, message) => {
                assert_ne!(id, self.1);
                self.2.update((self.0.internal_addr(id), message))
            }
            Message::ToAll(message) => {
                for id in 0..self.0.len() as u8 {
                    if id == self.1 {
                        continue;
                    }
                    self.2.update((self.0.internal_addr(id), message.clone()))
                }
            }
        }
    }
}

// type PeerTable = Table<public key, peer id>
