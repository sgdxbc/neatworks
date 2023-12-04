use std::net::{IpAddr, SocketAddr};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]

pub struct Config {
    pub seed: u64,
    pub num_host_peer: usize,
    pub hosts: Vec<IpAddr>,
    pub index: (usize, usize),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Peer {
    pub id: [u8; 32],
    pub key: Vec<u8>,
    pub addr: SocketAddr,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FindPeer {
    pub target: [u8; 32],
    pub count: usize,
}
