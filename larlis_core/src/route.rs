use std::{
    collections::{BTreeMap, HashMap},
    hash::Hash,
    net::{IpAddr, SocketAddr},
};

use rand::Rng;

#[derive(Default)]
pub struct Table<K, I> {
    hosts: BTreeMap<usize, IpAddr>,
    identities: Vec<K>,
    routes: HashMap<I, SocketAddr>,
}

pub type ClientTable = Table<u32, u32>;
// type PeerTable = Table<public key, peer id>

impl ClientTable {
    pub fn add_host(&mut self, host: IpAddr, count: usize, mut rng: impl Rng) {
        self.hosts.insert(self.routes.len(), host);
        for port in (50000..).take(count) {
            let identity = rng.gen();
            self.identities.push(identity);
            self.routes.insert(identity, SocketAddr::from((host, port)));
        }
    }
}

impl<K, I> Table<K, I> {
    pub fn addr(&self, index: usize) -> SocketAddr {
        let (&base_index, &host) = self.hosts.range(..=index).last().unwrap();
        SocketAddr::from((host, 50000 + (index - base_index) as u16))
    }

    pub fn identity(&self, index: usize) -> &K {
        &self.identities[index]
    }

    pub fn lookup_addr(&self, id: &I) -> SocketAddr
    where
        I: Eq + Hash,
    {
        self.routes[id]
    }
}
