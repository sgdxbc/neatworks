use std::{collections::HashSet, sync::Arc};

use borsh::{BorshDeserialize, BorshSerialize};
use ethnum::U256;

use crate::{
    channel::{EventSender, EventSource},
    crypto::{digest, Packet, Signer},
    task::BackgroundSpawner,
    transport::Addr,
    Transport,
};

pub type PeerId = [u8; 32];
pub type Location = [u8; 32];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PeerKey(secp256k1::PublicKey);

impl BorshSerialize for PeerKey {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        BorshSerialize::serialize(&self.0.serialize(), writer)
    }
}

impl BorshDeserialize for PeerKey {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> borsh::io::Result<Self> {
        Ok(Self(
            secp256k1::PublicKey::from_slice(&<[u8; 33] as BorshDeserialize>::deserialize_reader(
                reader,
            )?)
            .map_err(|err| borsh::io::Error::new(borsh::io::ErrorKind::InvalidInput, err))?,
        ))
    }
}

impl PeerKey {
    fn id(&self) -> crate::Result<PeerId> {
        digest(self)
    }
}

fn distance(id: &PeerId, target: &Location) -> U256 {
    U256::from_le_bytes(*id) ^ U256::from_le_bytes(*target)
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum Message {
    FindPeer(Packet),
    FindPeerOk(Packet),
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct FindPeer {
    target: Location,
    peer_key: PeerKey,
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct FindPeerOk {
    target: Location,
    closest: Vec<PeerRecord>,
    peer_key: PeerKey,
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
struct PeerRecord {
    id: PeerId, // redundant for a well-formed record
    key: PeerKey,
    addr: Addr,
}

#[derive(Debug)]
struct Peer {
    key: PeerKey,
    signer: Arc<Signer>,
    spawner: BackgroundSpawner,
}

async fn find_session(
    peer: Arc<Peer>,
    target: Location,
    local_closest: Vec<PeerRecord>,
    mut source: EventSource<FindPeerOk>,
    event: EventSender<Vec<PeerRecord>>,
    transport: impl Transport<Message>,
) -> crate::Result<()> {
    let mut contacted = HashSet::new();
    let mut contacting = HashSet::new();
    let mut closest = local_closest;
    for record in closest.iter().take(3) {
        peer.send_find_peer(&target, record.addr.clone(), transport.clone());
        contacting.insert(record.key);
    }
    if event.send(closest.clone()).is_err() {
        return Ok(());
    }
    loop {
        let message = tokio::select! {
            message = source.next() => message?,
            () = event.closed() => break,
        };
        if !contacting.remove(&message.peer_key) {
            continue;
        }
        contacted.insert(message.peer_key);
        for record in message.closest {
            let index = match closest
                .binary_search_by_key(&distance(&record.id, &target), |record| {
                    distance(&record.id, &target)
                }) {
                Err(index) => index,
                Ok(index) => {
                    assert_eq!(record, closest[index]);
                    continue;
                }
            };
            closest.insert(index, record)
        }
        if event.send(closest.clone()).is_err() {
            break;
        }
        let Some(record) = closest
            .iter()
            .take(20)
            .find(|record| !contacted.contains(&record.key))
        else {
            break;
        };
        peer.send_find_peer(&target, record.addr.clone(), transport.clone());
    }
    Ok(())
}

impl Peer {
    fn send_find_peer(&self, target: &Location, addr: Addr, transport: impl Transport<Message>) {
        let find_peer = FindPeer {
            target: *target,
            peer_key: self.key,
        };
        let signer = self.signer.clone();
        self.spawner.spawn(async move {
            transport
                .send_to(
                    addr,
                    Message::FindPeer(signer.serialize_sign(find_peer)?.into()),
                )
                .await?;
            Ok(())
        });
    }
}
