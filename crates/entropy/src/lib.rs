use borsh::{BorshDeserialize, BorshSerialize};
use halloween::{
    crypto::Packet,
    kademlia::{self, Location, PeerId},
    transport::Addr,
    Result,
};

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum Message {
    Kademlia(kademlia::Message),
    SendFragment(SendFragment),
    Publish(Packet),
    PublishOk(Packet),
    Query(Packet),
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
struct Publish {
    chunk: Location,
    id: PeerId,
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
struct PublishOk {
    chunk: Location,
    proof: (),
    id: PeerId,
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct SendFragment {
    chunk: Location,
    index: u32,
    // this should always be a SocketAddr, just for reusing borsh impl
    listener: Addr,
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
struct Query {
    chunk: Location,
    id: PeerId,
}

pub async fn put_session(_data: &[u8]) -> crate::Result<Vec<Location>> {
    Ok(Default::default())
}
