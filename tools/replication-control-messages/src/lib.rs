use std::{collections::HashMap, net::SocketAddr, ops::Range};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Client {
    pub id_range: Range<u32>,
    pub addr_book: AddrBook,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Replica {
    pub id: u8,
    pub addr_book: AddrBook,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddrBook {
    pub replica_addrs: HashMap<u8, SocketAddr>,
    pub client_addrs: HashMap<u32, SocketAddr>,
}
