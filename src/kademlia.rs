use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, SystemTime},
};

use borsh::{BorshDeserialize, BorshSerialize};
use ethnum::U256;
use tokio::task::JoinSet;

use crate::{
    channel::{EventSender, EventSource, SubscribeSource},
    crypto::{digest, Packet, Signer, Verifier},
    event_channel,
    task::BackgroundSpawner,
    transport::Addr,
    Transport,
};

pub type PeerId = [u8; 32];
pub type Location = [u8; 32];

fn peer_id(verifier: &Verifier) -> crate::Result<PeerId> {
    digest(verifier)
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
struct FindPeer {
    target: Location,
    count: usize,
    verifier: Verifier,
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
struct FindPeerOk {
    target: Location,
    closest: Vec<PeerRecord>,
    verifier: Verifier,
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct PeerRecord {
    pub id: PeerId, // redundant for a well-formed record
    pub verifier: Verifier,
    pub addr: Addr,
}

impl PeerRecord {
    pub fn new(signer: &Signer, addr: Addr) -> crate::Result<Self> {
        let verifier = Verifier::from(signer);
        Ok(Self {
            id: peer_id(&verifier)?,
            verifier,
            addr,
        })
    }
}

#[derive(Debug)]
pub struct Peer {
    pub verifier: Verifier,
    pub signer: Arc<Signer>,
    pub spawner: BackgroundSpawner,
}

async fn find_session(
    peer: Arc<Peer>,
    target: Location,
    count: usize,
    local_closest: Vec<PeerRecord>,
    mut source: EventSource<FindPeerOk>,
    event: EventSender<Vec<PeerRecord>>,
    transport: impl Transport<Message>,
) -> crate::Result<()> {
    let mut contacted = HashSet::new();
    let mut contacting = HashSet::new();
    let mut closest = local_closest;

    let find_peer = FindPeer {
        target,
        count,
        verifier: peer.verifier,
    };
    let find_peer = Message::FindPeer(peer.signer.serialize_sign(find_peer)?.into());
    let (ids, addrs) = closest
        .iter()
        .take(3)
        .map(|record| (record.id, record.addr.clone()))
        .unzip::<_, _, Vec<_>, Vec<_>>();
    transport
        .send_to_all(addrs.into_iter(), find_peer.clone())
        .await?;
    contacting.extend(ids);
    if event.send(closest.clone()).is_err() {
        return Ok(());
    }
    loop {
        let message = tokio::select! {
            message = source.next() => message?,
            () = event.closed() => break,
        };
        let peer_id = peer_id(&message.verifier)?;
        if !contacting.remove(&peer_id) {
            continue;
        }
        contacted.insert(peer_id);
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
            .take(count)
            .find(|record| !contacted.contains(&record.id))
        else {
            break;
        };
        transport
            .send_to(record.addr.clone(), find_peer.clone())
            .await?
    }
    Ok(())
}

#[derive(Debug)]
pub struct Buckets {
    center: PeerRecord,
    distances: Vec<Bucket>,
    expire_duration: Duration,
}

type Bucket = Vec<BucketRecord>;

#[derive(Debug, Clone)]
struct BucketRecord {
    peer: PeerRecord,
    expire: SystemTime,
}

impl Buckets {
    pub fn new(center: PeerRecord, expire_duration: Duration) -> Self {
        Self {
            center,
            distances: vec![Default::default(); 255],
            expire_duration,
        }
    }

    fn index(&self, location: &Location) -> usize {
        distance(&self.center.id, location).leading_zeros() as _
    }

    pub fn insert(&mut self, record: PeerRecord) {
        let index = self.index(&record.id);
        let bucket = &mut self.distances[index];
        if bucket.len() == 20 {
            if let Some(bucket_index) = bucket
                .iter()
                .position(|record| record.expire.elapsed().is_ok())
            {
                bucket.remove(bucket_index);
            } else {
                return;
            }
        }
        bucket.push(BucketRecord {
            peer: record,
            expire: SystemTime::now() + self.expire_duration,
        })
    }

    fn find_closest(&self, target: &Location, count: usize) -> Vec<PeerRecord> {
        let mut records = Vec::new();
        let index = self.index(target);
        for index in (index..U256::BITS as _).chain((0..index).rev()) {
            let mut bucket_records = self.distances[index]
                .iter()
                .map(|record| record.peer.clone())
                .collect::<Vec<_>>();
            bucket_records.sort_unstable_by_key(|record| distance(&record.id, target));
            records.extend(bucket_records.into_iter().take(count - records.len()));
            assert!(records.len() <= count);
            if records.len() == count {
                break;
            }
        }
        assert!(records
            .windows(2)
            .all(|records| distance(&records[0].id, target) < distance(&records[1].id, target)));
        records
    }
}

pub async fn session(
    peer: Arc<Peer>,
    mut buckets: Buckets,
    mut subscribe_source: SubscribeSource<(Location, usize), Vec<PeerRecord>>,
    mut message_source: EventSource<(Addr, Message)>,
    spawner: BackgroundSpawner,
    transport: impl Transport<Message>,
) -> crate::Result<()> {
    let mut verify_find_peer_sessions = JoinSet::new();
    let mut verify_find_peer_ok_sessions = JoinSet::new();
    let mut find_handles = HashMap::new();
    let mut close_find_handle_sessions = JoinSet::new();

    loop {
        type Subscribe = Option<((Location, usize), EventSender<Vec<PeerRecord>>)>;
        enum Select {
            Subscribe(Subscribe),
            Message((Addr, Message)),
            VerifiedFindPeer(crate::Result<(Addr, FindPeer)>),
            VerifiedFindPeerOk(crate::Result<(Addr, FindPeerOk)>),
            RemoveFindHandle(Location),
        }
        match tokio::select! {
            subscribe = subscribe_source.option_next() => Select::Subscribe(subscribe),
            message = message_source.next() => Select::Message(message?),
            Some(verified) = verify_find_peer_sessions.join_next()
                => Select::VerifiedFindPeer(verified?),
            Some(verified) = verify_find_peer_ok_sessions.join_next()
                => Select::VerifiedFindPeerOk(verified?),
            Some(target) = close_find_handle_sessions.join_next()
                => Select::RemoveFindHandle(target??)
        } {
            Select::Subscribe(None) => break Ok(()),
            Select::Subscribe(Some(((target, count), event))) => {
                let (message_event, message_source) = event_channel();
                let evicted = find_handles.insert(target, message_event);
                if evicted.is_some() {
                    crate::bail!("duplicated find session {target:02x?}")
                }
                let find = spawner.spawn(find_session(
                    peer.clone(),
                    target,
                    count,
                    buckets.find_closest(&target, count),
                    message_source,
                    event,
                    transport.clone(),
                ));
                close_find_handle_sessions.spawn(async move {
                    find.await?;
                    Ok::<_, crate::Error>(target)
                });
            }
            Select::Message((remote, Message::FindPeer(message))) => {
                verify_find_peer_sessions.spawn(async move {
                    let mut message = message.deserialize::<FindPeer>()?;
                    message.verifier.clone().verify(&mut message)?;
                    Ok::<_, crate::Error>((remote, message.inner))
                });
            }
            Select::Message((remote, Message::FindPeerOk(message))) => {
                verify_find_peer_ok_sessions.spawn(async move {
                    let mut message = message.deserialize::<FindPeerOk>()?;
                    message.verifier.clone().verify(&mut message)?;
                    Ok::<_, crate::Error>((remote, message.inner))
                });
            }
            Select::VerifiedFindPeer(Err(_)) => {}
            Select::VerifiedFindPeer(Ok((remote, message))) => {
                let record = PeerRecord {
                    id: peer_id(&message.verifier)?,
                    verifier: message.verifier,
                    addr: remote.clone(),
                };
                buckets.insert(record);
                let find_peer_ok = FindPeerOk {
                    target: message.target,
                    closest: buckets.find_closest(&message.target, message.count),
                    verifier: peer.verifier,
                };
                let signer = peer.signer.clone();
                let transport = transport.clone();
                spawner.spawn(async move {
                    transport
                        .send_to(
                            remote,
                            Message::FindPeerOk(signer.serialize_sign(find_peer_ok)?.into()),
                        )
                        .await?;
                    Ok(())
                });
            }
            Select::VerifiedFindPeerOk(Err(_)) => {}
            Select::VerifiedFindPeerOk(Ok((remote, message))) => {
                let record = PeerRecord {
                    id: peer_id(&message.verifier)?,
                    verifier: message.verifier,
                    addr: remote.clone(),
                };
                buckets.insert(record);
                if let Some(event) = find_handles.get(&message.target) {
                    let _ = event.send(message);
                }
            }
            Select::RemoveFindHandle(target) => {
                let event = find_handles.remove(&target).unwrap();
                assert!(event.0.is_closed());
            }
        }
    }
}
