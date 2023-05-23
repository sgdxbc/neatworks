// design choice: unreliable network torlarance, i.e. resending
// `(Pre)Prepare` and `Commit` messages serve dual purposes: a local certificate
// for certain proposal (as defined in paper), and a demand indicator.
// That is, (re)sending `(Pre)Prepare` also means sender is querying `Prepare`,
// and (re)sending `Commit` also means sender is querying `Commit`.
use std::collections::HashMap;

use bincode::Options;
use neat_core::{
    actor::State,
    message::{
        Lift,
        Timeout::{Set, Unset},
    },
};
use serde::{Deserialize, Serialize};

use crate::{Reply, Request};

type ViewNum = u16;
type OpNum = u32;

pub struct Upcall<'o> {
    view_num: ViewNum,
    op_num: OpNum,
    client_id: u32,
    request_num: u32,
    op: &'o [u8],
}

pub struct AppLift {
    replica_id: u8,
    replies: HashMap<u32, Reply>,
}

impl AppLift {
    pub fn new(replica_id: u8) -> Self {
        Self {
            replica_id,
            replies: Default::default(),
        }
    }
}

impl<A> Lift<A, Upcall<'_>> for AppLift
where
    A: neat_core::App + 'static,
{
    type Out<'a> = Option<(u32, Reply)>;

    fn update<'a>(&'a mut self, state: &'a mut A, message: Upcall<'_>) -> Self::Out<'a> {
        match self.replies.get(&message.client_id) {
            Some(reply) if reply.request_num > message.request_num => return None,
            Some(reply) if reply.request_num == message.request_num => {
                return Some((message.client_id, reply.clone()))
            }
            _ => {}
        }
        assert_ne!(message.view_num, ViewNum::MAX);
        assert_ne!(message.op_num, 0);
        let result = state.update(message.op_num, message.op);
        let reply = Reply {
            replica_id: self.replica_id,
            view_num: message.view_num,
            request_num: message.request_num,
            result,
        };
        self.replies.insert(message.client_id, reply.clone());
        Some((message.client_id, reply))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToReplica {
    Request(Request),
    PrePrepare(PrePrepare, Vec<Request>),
    Prepare(Prepare),
    Commit(Commit),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrePrepare {
    pub(crate) view_num: ViewNum,
    op_num: OpNum,
    digest: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prepare {
    view_num: ViewNum,
    op_num: OpNum,
    digest: [u8; 32],
    pub(crate) replica_id: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Commit {
    view_num: ViewNum,
    op_num: OpNum,
    digest: [u8; 32],
    pub(crate) replica_id: u8,
}

pub type Egress = neat_core::message::Egress<u8, ToReplica>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum TimeoutEvent {
    // view change
    Prepare(ViewNum, OpNum),
    Commit(ViewNum, OpNum),
}

pub type Timeout = neat_core::message::Timeout<TimeoutEvent>;

pub struct Replica<U, E, T> {
    id: u8,
    n: usize,
    f: usize,

    view_num: ViewNum,
    op_num: OpNum,
    // client id -> highest request number that has been working on (initially 0)
    client_table: HashMap<u32, u32>,
    log: Vec<Vec<Request>>, // save it locally or send it to external actor?
    // pre_prepares, prepared and committed get cleared when entering every view
    pre_prepares: HashMap<OpNum, (PrePrepare, Vec<Request>)>,
    prepared: HashMap<OpNum, HashMap<u8, Prepare>>,
    committed: HashMap<OpNum, HashMap<u8, Commit>>,
    execute_number: OpNum,

    pub upcall: U,
    pub egress: E,
    pub timeout: T,
}

impl<U, E, T> Replica<U, E, T> {
    pub fn new(id: u8, n: usize, f: usize, upcall: U, egress: E, timeout: T) -> Self {
        #[allow(clippy::int_plus_one)] // to conform conventional statement
        {
            assert!(n >= 3 * f + 1);
        }
        Self {
            id,
            n,
            f,
            view_num: 0,
            op_num: 0,
            client_table: Default::default(),
            log: Default::default(),
            pre_prepares: Default::default(),
            prepared: Default::default(),
            committed: Default::default(),
            execute_number: 0,
            upcall,
            egress,
            timeout,
        }
    }
}

impl<U, E, T> State<ToReplica> for Replica<U, E, T>
where
    U: for<'m> State<Upcall<'m>>,
    E: State<Egress>,
    T: State<Timeout>,
{
    fn update(&mut self, message: ToReplica) {
        match message {
            ToReplica::Request(message) => self.handle_request(message),
            ToReplica::PrePrepare(message, requests) => self.handle_pre_prepare(message, requests),
            ToReplica::Prepare(message) => self.handle_prepare(message),
            ToReplica::Commit(message) => self.handle_commit(message),
        }
    }
}

impl<U, E, T> State<TimeoutEvent> for Replica<U, E, T>
where
    U: for<'m> State<Upcall<'m>>,
    E: State<Egress>,
    T: State<Timeout>,
{
    fn update(&mut self, message: TimeoutEvent) {
        match message {
            TimeoutEvent::Prepare(view_num, op_num) => {
                assert_eq!(self.view_num, view_num);
                assert!(self.prepared_slot(op_num).is_none());
                //
                if self.id == self.primary_id() {
                    self.send_pre_prepare(op_num)
                } else {
                    self.send_prepare(op_num, Egress::ToAll)
                }
                self.timeout
                    .update(Set(TimeoutEvent::Prepare(view_num, op_num)))
            }
            TimeoutEvent::Commit(view_num, op_num) => {
                assert_eq!(self.view_num, view_num);
                assert!(self.committed_slot(op_num).is_none());
                //
                self.send_commit(op_num, Egress::ToAll);
                self.timeout
                    .update(Set(TimeoutEvent::Commit(view_num, op_num)))
            }
        }
    }
}

impl<U, E, T> Replica<U, E, T> {
    fn primary_id(&self) -> u8 {
        (self.view_num as usize % self.n) as _
    }

    fn prepared_slot(&self, op_num: OpNum) -> Option<&[Request]> {
        let Some((_, requests)) = self.pre_prepares.get(&op_num) else {
            return None;
        };
        if self
            .prepared
            .get(&op_num)
            .unwrap_or(&Default::default())
            .len()
            + 1 // self vote is implied
            >= self.n - self.f
        {
            Some(requests)
        } else {
            None
        }
    }

    fn committed_slot(&self, op_num: OpNum) -> Option<&[Request]> {
        let Some(slot) = self.prepared_slot(op_num) else {
            return None;
        };
        if self
            .committed
            .get(&op_num)
            .unwrap_or(&Default::default())
            .len()
            + 1 // self vote is implied
            >= self.n - self.f
        {
            Some(slot)
        } else {
            None
        }
    }
}

fn digest(requests: &[Request]) -> [u8; 32] {
    use sha2::Digest;
    sha2::Sha256::digest(bincode::options().serialize(requests).unwrap()).into()
}

impl<U, E, T> Replica<U, E, T>
where
    U: for<'m> State<Upcall<'m>>,
    E: State<Egress>,
    T: State<Timeout>,
{
    fn handle_request(&mut self, message: Request) {
        if self.id != self.primary_id() {
            // TODO relay + view change timer
            return;
        }

        match self.client_table.get(&message.client_id) {
            Some(request_num) if request_num > &message.request_num => {
                self.upcall.update(Upcall {
                    view_num: ViewNum::MAX,
                    op_num: 0,
                    client_id: message.client_id,
                    request_num: message.request_num,
                    op: &message.op,
                });
                return;
            }
            Some(request_num) if request_num == &message.request_num => return,
            _ => {}
        }

        self.op_num += 1;
        self.client_table
            .insert(message.client_id, message.request_num);

        if self.n == 1 {
            todo!("single replica setup")
        }

        // TODO batching
        let requests = vec![message];
        let pre_prepare = PrePrepare {
            view_num: self.view_num,
            op_num: self.op_num,
            digest: digest(&requests),
        };
        self.pre_prepares
            .insert(self.op_num, (pre_prepare, requests));
        self.send_pre_prepare(self.op_num);
        self.timeout
            .update(Set(TimeoutEvent::Prepare(self.view_num, self.op_num)));
        assert!(!self.prepared.contains_key(&self.op_num));
        assert!(!self.committed.contains_key(&self.op_num));
    }

    fn handle_pre_prepare(&mut self, message: PrePrepare, requests: Vec<Request>) {
        if message.view_num < self.view_num {
            return;
        }
        if message.view_num > self.view_num {
            // TODO
            self.view_num = message.view_num;
        }
        if self.pre_prepares.contains_key(&message.op_num) {
            // dedicated reply to late sender
            self.send_prepare(message.op_num, Egress::to(self.primary_id()));
            return;
        }
        // TODO watermark

        let op_num = message.op_num;
        let digest = message.digest;
        self.pre_prepares
            .insert(message.op_num, (message, requests));
        if let Some(prepared) = self.prepared.get_mut(&op_num) {
            prepared.retain(|_, prepare| prepare.digest == digest);
        }
        if let Some(committed) = self.committed.get_mut(&op_num) {
            committed.retain(|_, commit| commit.digest == digest);
        }

        self.send_prepare(op_num, Egress::ToAll);
        self.timeout
            .update(Set(TimeoutEvent::Prepare(self.view_num, op_num)));
        // `pre_prepares` implies the insertion
        // self.insert_prepare(prepare);
    }

    fn handle_prepare(&mut self, message: Prepare) {
        if message.view_num < self.view_num {
            return;
        }
        if message.view_num > self.view_num {
            // TODO
            self.view_num = message.view_num;
        }
        if self.prepared_slot(message.op_num).is_some() {
            // dedicated reply to late sender
            self.send_prepare(message.op_num, Egress::to(message.replica_id));
            return;
        }
        if let Some((pre_prepare, _)) = self.pre_prepares.get(&message.op_num) {
            if message.digest != pre_prepare.digest {
                //
                return;
            }
        }

        self.insert_prepare(message);
    }

    fn handle_commit(&mut self, message: Commit) {
        if message.view_num < self.view_num {
            return;
        }
        if message.view_num > self.view_num {
            // TODO
            self.view_num = message.view_num;
        }
        if self.committed_slot(message.op_num).is_some() {
            // dedicated reply to late sender
            self.send_commit(message.op_num, Egress::to(message.replica_id));
            return;
        }
        if let Some((pre_prepare, _)) = self.pre_prepares.get(&message.op_num) {
            if message.digest != pre_prepare.digest {
                //
                return;
            }
        }

        self.insert_commit(message);
    }

    fn send_pre_prepare(&mut self, op_num: OpNum) {
        assert_eq!(self.id, self.primary_id());
        let (pre_prepare, requests) = self.pre_prepares[&op_num].clone();
        self.egress
            .update(Egress::ToAll(ToReplica::PrePrepare(pre_prepare, requests)));
    }

    fn send_prepare(&mut self, op_num: OpNum, to_whom: impl FnOnce(ToReplica) -> Egress) {
        assert_ne!(self.id, self.primary_id());
        let prepare = Prepare {
            view_num: self.view_num,
            op_num,
            digest: self.pre_prepares[&op_num].0.digest,
            replica_id: self.id,
        };
        self.egress.update(to_whom(ToReplica::Prepare(prepare)));
    }

    fn send_commit(&mut self, op_num: OpNum, to_whom: impl FnOnce(ToReplica) -> Egress) {
        let commit = Commit {
            view_num: self.view_num,
            op_num,
            digest: self.pre_prepares[&op_num].0.digest,
            replica_id: self.id,
        };
        self.egress.update(to_whom(ToReplica::Commit(commit)));
    }

    fn insert_prepare(&mut self, prepare: Prepare) {
        assert!(self.prepared_slot(prepare.op_num).is_none());
        let op_num = prepare.op_num;
        self.prepared
            .entry(op_num)
            .or_default()
            .insert(prepare.replica_id, prepare);
        if self.prepared_slot(op_num).is_some() {
            self.send_commit(op_num, Egress::ToAll);
            self.timeout
                .update(Unset(TimeoutEvent::Prepare(self.view_num, op_num)));
            self.timeout
                .update(Set(TimeoutEvent::Commit(self.view_num, op_num)));
            // `pre_prepares` implies the insertion
            // self.insert_commit(commit);

            // TODO adaptive batching
        }
    }

    fn insert_commit(&mut self, commit: Commit) {
        assert!(self.committed_slot(commit.op_num).is_none());
        let op_num = commit.op_num;
        self.committed
            .entry(op_num)
            .or_default()
            .insert(commit.replica_id, commit);
        if self.committed_slot(op_num).is_some() {
            self.timeout
                .update(Unset(TimeoutEvent::Commit(self.view_num, op_num)));
            self.execute();
        }
    }

    fn execute(&mut self) {
        while let Some(requests) = self.committed_slot(self.execute_number + 1) {
            // alternative: move `requests` from `pre_prepares` to `log`
            let requests = requests.to_vec();
            self.log.push(requests.clone());
            for request in requests {
                let upcall = Upcall {
                    view_num: self.view_num,
                    op_num: self.execute_number,
                    client_id: request.client_id,
                    request_num: request.request_num,
                    op: &request.op,
                };
                self.upcall.update(upcall);
            }
            self.execute_number += 1;
        }
    }
}
