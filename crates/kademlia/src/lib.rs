pub mod store;

use std::{
    collections::{BTreeMap, HashMap},
    mem::replace,
    time::{Duration, SystemTime},
};

use derive_more::From;
use neat::{
    context::{Addr, MultiplexReceive, TimerId, To},
    crypto::{Sign, Signed, VerifyingKey},
    Context,
};
use primitive_types::U256;
use serde::{Deserialize, Serialize};

use crate::store::{distance, Store};

pub type PeerId = [u8; 32];

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq, Hash, From, Serialize, Deserialize)]
pub enum Message {
    // p2p messages
    Find(Signed<PeerRecord>),
    FindOk(Signed<FindOk>),
    // rpc messages
    Query(Query),
    CancelQuery(CancelQuery),
    QueryStatus(QueryStatus),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct PeerRecord {
    id: PeerId,
    verifying_key: VerifyingKey,
    addr: Addr,
    last_active: SystemTime,
    target: PeerId,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FindOk {
    peer_id: PeerId,
    target: PeerId,
    peer_records: Vec<Signed<PeerRecord>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Query {
    target: PeerId,
    num_peer: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CancelQuery {
    target: PeerId,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct QueryStatus {
    target: PeerId,
    current_closest: Vec<Signed<PeerRecord>>,
    finished: bool,
}

#[derive(Debug)]
pub struct State {
    context: Context<Message>,
    id: PeerId,
    verifying_key: VerifyingKey,
    num_parallel: usize,

    store: Store,
    queries: HashMap<PeerId, QueryState>,
}

#[derive(Debug)]
struct QueryState {
    remote: Addr,
    num_peer: usize,
    peers: BTreeMap<U256, (Signed<PeerRecord>, Contact)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Contact {
    Pending,
    Sent(TimerId),
    Replied,
    Timeout,
}

impl MultiplexReceive for State {
    type Message = Message;

    fn handle(&mut self, receiver: Addr, remote: Addr, message: Self::Message) {
        assert_eq!(receiver, self.context.addr());
        match message {
            Message::Query(message) => self.query(remote, message.target, message.num_peer),
            Message::CancelQuery(message) => self.cancel_query(&message.target),
            Message::QueryStatus(_) => unimplemented!(),
            Message::Find(message) => self.handle_find(remote, message),
            Message::FindOk(message) => self.handle_find_ok(remote, message),
        }
    }

    fn on_timer(&mut self, receiver: Addr, id: TimerId) {
        assert_eq!(receiver, self.context.addr());
        for (&target, query) in &mut self.queries {
            for (_, contact) in query.peers.values_mut() {
                if *contact == Contact::Sent(id) {
                    *contact = Contact::Timeout;
                    self.do_find(&target);
                    return;
                }
            }
        }
    }
}

impl State {
    pub fn query(&mut self, remote: Addr, target: PeerId, num_peer: usize) {
        let records = self.store.closest(&target, self.store.bucket_size);
        let query = QueryState {
            remote,
            num_peer,
            peers: records
                .iter()
                .map(|record| {
                    (
                        distance(&record.id, &target),
                        (record.clone(), Contact::Pending),
                    )
                })
                .collect(),
        };
        let evicted = self.queries.insert(target, query);
        assert!(evicted.is_none());
        let status = QueryStatus {
            target,
            current_closest: records.into_iter().take(num_peer).collect(),
            finished: false,
        };
        self.context.send(To::Addr(remote), status);
        for _ in 0..self.num_parallel {
            self.do_find(&target)
        }
    }

    pub fn cancel_query(&mut self, target: &PeerId) {
        self.queries.remove(target).unwrap();
    }

    fn do_find(&mut self, target: &PeerId) {
        let query = self.queries.get_mut(target).unwrap();
        for (record, contact) in query.peers.values_mut() {
            if matches!(contact, Contact::Pending) {
                let find = PeerRecord {
                    target: *target,
                    id: self.id,
                    verifying_key: self.verifying_key,
                    addr: self.context.addr(),
                    last_active: SystemTime::now(),
                };
                self.context.send(To::Addr(record.addr), find);
                *contact = Contact::Sent(self.context.set(Duration::from_millis(500)));
                return;
            }
        }
        unimplemented!()
    }

    fn handle_find(&mut self, remote: Addr, message: Signed<PeerRecord>) {
        self.store.insert(message.clone());
        let peer_records = self.store.closest(&message.target, self.store.bucket_size);
        let find_ok = FindOk {
            target: message.target,
            peer_id: self.id,
            peer_records,
        };
        self.context.send(To::Addr(remote), find_ok)
    }

    fn handle_find_ok(&mut self, _remote: Addr, message: Signed<FindOk>) {
        let Some(query) = self.queries.get_mut(&message.target) else {
            return;
        };
        let message = message.inner;
        for record in message.peer_records {
            self.store.insert(record.clone());
            let distance = distance(&record.id, &message.target);
            if let Some((exist_record, _)) = query.peers.get(&distance) {
                assert_eq!(exist_record.id, record.id)
            } else {
                query.peers.insert(distance, (record, Contact::Pending));
            }
        }

        let (record, contact) = query
            .peers
            .get_mut(&distance(&message.peer_id, &message.target))
            .unwrap();
        assert_eq!(record.id, message.peer_id);
        if let Contact::Sent(timer_id) = replace(contact, Contact::Replied) {
            self.context.unset(timer_id)
        }

        let replied_count = query
            .peers
            .values()
            .filter(|(_, contact)| !matches!(contact, Contact::Timeout))
            .take_while(|(_, contact)| matches!(contact, Contact::Replied))
            .take(query.num_peer)
            .count();
        let finished = replied_count == query.num_peer;

        let status = QueryStatus {
            target: message.target,
            current_closest: query
                .peers
                .values()
                .map(|(record, _)| record.clone())
                .take(query.num_peer)
                .collect(),
            finished,
        };
        self.context.send(To::Addr(query.remote), status);

        if !finished {
            self.do_find(&message.target)
        } else {
            self.queries.remove(&message.target).unwrap();
        }
    }
}

impl Sign<PeerRecord> for Message {
    fn sign(message: PeerRecord, signer: &neat::crypto::Signer) -> Self {
        Message::Find(signer.sign_public_for_batch(message))
    }
}

impl Sign<FindOk> for Message {
    fn sign(message: FindOk, signer: &neat::crypto::Signer) -> Self {
        Message::FindOk(signer.sign_public(message))
    }
}
