use std::{
    collections::{BTreeMap, HashMap},
    net::SocketAddr,
};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Replica {
    pub addr: SocketAddr,
    pub id: u8,
    pub addr_book: AddrBook,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddrBook {
    pub replica_addrs: HashMap<u8, SocketAddr>,
    pub client_addrs: BTreeMap<u32, SocketAddr>,
}
