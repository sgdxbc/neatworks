pub mod client;
pub mod crypto;
pub mod replica;

pub use client::Client;
pub use crypto::{Sign, Signature, Verify};
pub use replica::{AppLift, Replica, ToReplica};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    client_id: u32,
    request_num: u32,
    op: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reply {
    request_num: u32,
    result: Vec<u8>,
    replica_id: u8,
    view_num: u16,
}
