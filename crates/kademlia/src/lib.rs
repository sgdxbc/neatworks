use serde::{Deserialize, Serialize};

pub type PeerId = [u8; 32];

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Message {
    // p2p messages
    Find(Find),
    FindOk(FindOk),
    // rpc messages
    Query(Query),
    CancelQuery(CancelQuery),
    QueryStatus(QueryStatus),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Find {
    target: PeerId,
    peer_id: PeerId,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FindOk {
    target: PeerId,
    //
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Query {
    query_num: u32,
    target: PeerId,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CancelQuery {
    query_num: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct QueryStatus {
    query_num: u32,
    //
}
