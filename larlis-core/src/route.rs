use std::{
    collections::{BTreeMap, HashMap},
    net::{IpAddr, SocketAddr},
};

use rand::Rng;

#[derive(Default)]
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

pub struct ReplicaTable {
    identities: Vec<[u8; 32]>,
    routes: Vec<SocketAddr>,
}

impl ReplicaTable {
    pub fn len(&self) -> usize {
        self.identities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.identities.is_empty()
    }

    pub fn add(&mut self, addr: SocketAddr, mut rng: impl Rng) {
        let index = self.len();
        self.identities.push(rng.gen());
        self.routes.insert(index as _, addr);
    }

    pub fn identity(&self, id: u8) -> [u8; 32] {
        self.identities[id as usize]
    }

    pub fn lookup_addr(&self, id: u8) -> SocketAddr {
        self.routes[id as usize]
    }
}

// type PeerTable = Table<public key, peer id>
