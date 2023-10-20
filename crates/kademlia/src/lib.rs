pub mod store;

use neat::context::crypto::Signed;
use serde::{Deserialize, Serialize};
use store::PeerRecord;

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
    peer_record: Box<Signed<PeerRecord>>, // to avoid large variant size difference
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FindOk {
    target: PeerId,
    peer_records: Vec<Signed<PeerRecord>>,
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
    current_closest: Vec<Signed<PeerRecord>>,
    finished: bool,
}
