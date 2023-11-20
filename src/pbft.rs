use std::{collections::HashMap, future::Future, iter::repeat, sync::Arc};

use borsh::{BorshDeserialize, BorshSerialize};
use derive_more::From;
use tokio::{
    sync::Semaphore,
    task::JoinHandle,
    time::{timeout_at, Instant},
};

use crate::{
    crypto::{digest, Digest, Message, Packet},
    model::{
        event_channel, promise_channel, EventSender, EventSource, PromiseSender, SubmitSource,
        Transport,
    },
    Client, Replica,
};

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Request {
    client_id: u32,
    request_num: u32,
    op: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Reply {
    request_num: u32,
    result: Vec<u8>,
    replica_id: u8,
    view_num: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct PrePrepare {
    view_num: u32,
    op_num: u32,
    digest: Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Prepare {
    view_num: u32,
    op_num: u32,
    digest: Digest,
    replica_id: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Commit {
    view_num: u32,
    op_num: u32,
    digest: Digest,
    replica_id: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum ToReplica {
    Request(Request),
    PrePrepare(Packet, Vec<Request>),
    Prepare(Packet),
    Commit(Packet),
}

pub async fn client_session(
    client: Arc<Client>,
    mut receiver: SubmitSource<Vec<u8>, Vec<u8>>,
    mut source: EventSource<Reply>,
    transport: impl Transport<Request>,
) -> crate::Result<()> {
    let mut request_num = 0;
    let mut primary_replica = 0;

    while let Some((op, result)) = receiver.option_next().await {
        request_num += 1;
        let request = Request {
            client_id: client.id,
            request_num,
            op,
        };
        result.resolve(
            request_session(
                &client,
                request,
                &mut primary_replica,
                &mut source,
                &transport,
            )
            .await?,
        )?
    }
    Ok(())
}

async fn request_session(
    client: &Client,
    request: Request,
    primary_replica: &mut u8,
    source: &mut EventSource<Reply>,
    transport: &impl Transport<Request>,
) -> crate::Result<Vec<u8>> {
    let mut replies = HashMap::new();

    transport
        .send_to(
            client.addr_book.replica_addr(*primary_replica),
            request.clone(),
        )
        .await?;
    loop {
        let deadline = Instant::now() + client.retry_interval;
        while let Ok(reply) = timeout_at(deadline, source.next()).await {
            let reply = reply?;
            assert!(reply.request_num <= request.request_num);
            if reply.request_num == request.request_num {
                replies.insert(reply.replica_id, reply.clone());
                if replies
                    .values()
                    .filter(|matched_reply| matched_reply.result == reply.result)
                    .count()
                    == client.num_faulty + 1
                {
                    *primary_replica = (reply.view_num as usize % client.num_replica) as _;
                    return Ok(reply.result);
                }
            }
        }
        transport
            .send_to_all(client.addr_book.replica_addrs(), request.clone())
            .await?
    }
}

#[derive(Debug, From)]
pub enum ReplicaEvent {
    Message(ToReplica),
    ReplyReady(u32, Reply),
}

pub async fn replica_session(
    replica: Arc<Replica>,
    event: EventSender<ReplicaEvent>,
    mut source: EventSource<ReplicaEvent>,
    transport: impl Transport<ToReplica>,
    reply_transport: impl Transport<Reply>,
) -> crate::Result<()> {
    for view_num in 0.. {
        ViewState::new(
            replica.clone(),
            view_num,
            event.clone(),
            &mut source,
            transport.clone(),
            reply_transport.clone(),
        )
        .session()
        .await?;
        // TODO view change
    }
    Ok(())
}

async fn batch_session(
    permits: Arc<Semaphore>,
    mut source: EventSource<Request>,
) -> crate::Result<Vec<Request>> {
    let mut requests = vec![source.next().await?];
    loop {
        tokio::select! {
            permit = permits.acquire() => {
                permit?.forget();
                break Ok(requests);
            }
            request = source.next(), if requests.len() < 100 => requests.push(request?),
        }
    }
}

type Quorum<T> = HashMap<u8, Message<T>>;

#[derive(Debug)]
struct LogEntry {
    requests: Vec<Request>,
    #[allow(unused)]
    pre_prepare: Message<PrePrepare>,
    #[allow(unused)]
    prepares: Quorum<Prepare>,
    commits: Quorum<Commit>,
}

async fn prepare_session(
    replica: Arc<Replica>,
    pre_prepare: Message<PrePrepare>,
    requests: Vec<Request>,
    mut prepare_source: EventSource<Message<Prepare>>,
    commit_digest: PromiseSender<Digest>,
    transport: impl Transport<ToReplica>,
) -> crate::Result<LogEntry> {
    assert!(pre_prepare.verified);
    let mut prepares = HashMap::new();
    if !replica.is_primary(pre_prepare.view_num) {
        let prepare = Prepare {
            view_num: pre_prepare.view_num,
            op_num: pre_prepare.op_num,
            digest: pre_prepare.digest,
            replica_id: replica.id,
        };
        let prepare = replica.signer.serialize_sign(prepare)?;
        prepares.insert(replica.id, prepare.clone());
        replica.send_to_all_replica(ToReplica::Prepare(prepare.into()), transport);
    }
    while prepares.len() < 2 * replica.num_faulty {
        let mut prepare = prepare_source.next().await?;
        assert_eq!(
            prepare.op_num, pre_prepare.op_num,
            "incorrect dispatching of {prepare:?}"
        );
        if replica.verifiers[&prepare.replica_id]
            .verify(&mut prepare)
            .is_ok()
            && prepare.view_num == pre_prepare.view_num
            && prepare.digest == pre_prepare.digest
        {
            prepares.insert(prepare.replica_id, prepare);
        }
    }
    commit_digest.resolve(pre_prepare.digest)?;
    Ok(LogEntry {
        requests,
        pre_prepare,
        prepares,
        commits: Default::default(),
    })
}

async fn primary_prepare_session(
    replica: Arc<Replica>,
    view_num: u32,
    op_num: u32,
    requests: Vec<Request>,
    prepare_source: EventSource<Message<Prepare>>,
    commit_digest: PromiseSender<Digest>,
    transport: impl Transport<ToReplica>,
) -> crate::Result<LogEntry> {
    assert!(replica.is_primary(view_num));
    let pre_prepare = PrePrepare {
        view_num,
        op_num,
        digest: digest(&requests)?,
    };
    let pre_prepare = replica.signer.serialize_sign(pre_prepare)?;
    replica.send_to_all_replica(
        ToReplica::PrePrepare(pre_prepare.clone().into(), requests.clone()),
        transport.clone(),
    );
    prepare_session(
        replica,
        pre_prepare,
        requests,
        prepare_source,
        commit_digest,
        transport,
    )
    .await
}

async fn backup_prepare_session(
    replica: Arc<Replica>,
    view_num: u32,
    mut pre_prepare_source: EventSource<(Message<PrePrepare>, Vec<Request>)>,
    prepare_source: EventSource<Message<Prepare>>,
    commit_digest: PromiseSender<Digest>,
    transport: impl Transport<ToReplica>,
) -> crate::Result<LogEntry> {
    loop {
        let (mut pre_prepare, requests) = pre_prepare_source.next().await?;
        if replica.verifiers[&replica.primary(pre_prepare.view_num)]
            .verify(&mut pre_prepare)
            .is_ok()
            && pre_prepare.view_num == view_num
            && pre_prepare.digest == digest(&requests)?
        {
            return prepare_session(
                replica,
                pre_prepare,
                requests,
                prepare_source,
                commit_digest,
                transport,
            )
            .await;
        }
    }
}

async fn commit_session(
    replica: Arc<Replica>,
    view_num: u32,
    op_num: u32,
    digest: impl Future<Output = crate::Result<Digest>>,
    mut source: EventSource<Message<Commit>>,
    transport: impl Transport<ToReplica>,
) -> crate::Result<Quorum<Commit>> {
    let digest = digest.await?;
    let mut commits = HashMap::new();
    let commit = Commit {
        view_num,
        op_num,
        digest,
        replica_id: replica.id,
    };
    let commit = replica.signer.serialize_sign(commit)?;
    commits.insert(replica.id, commit.clone());
    replica.send_to_all_replica(ToReplica::Commit(commit.into()), transport);
    while commits.len() <= 2 * replica.num_faulty {
        let mut commit = source.next().await?;
        assert_eq!(commit.op_num, op_num, "incorrect dispatching of {commit:?}");
        if replica.verifiers[&commit.replica_id]
            .verify(&mut commit)
            .is_ok()
            && commit.view_num == view_num
            && commit.digest == digest
        {
            commits.insert(commit.replica_id, commit);
        }
    }
    Ok(commits)
}

#[derive(Debug)]
struct ViewState<'a, T, U> {
    replica: Arc<Replica>,
    view_num: u32,
    event: EventSender<ReplicaEvent>,
    source: &'a mut EventSource<ReplicaEvent>,
    transport: T,
    reply_transport: U,

    client_entires: HashMap<u32, ClientEntry>,
    batch_state: Option<BatchState>,
    propose_op: u32,
    prepare_op: u32,
    commit_op: u32,
    quorum_states: HashMap<u32, QuorumState>,
    log_entries: HashMap<u32, LogEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ClientEntry {
    Committing(u32),
    Committed(Reply),
}

#[derive(Debug)]
struct BatchState {
    event: EventSender<Request>,
    session: JoinHandle<crate::Result<Vec<Request>>>,
    permits: Arc<Semaphore>,
}

#[derive(Debug)]
struct QuorumState {
    prepare_session: JoinHandle<crate::Result<LogEntry>>,
    commit_session: JoinHandle<crate::Result<Quorum<Commit>>>,
    pre_prepare_event: Option<EventSender<(Message<PrePrepare>, Vec<Request>)>>,
    prepare_event: EventSender<Message<Prepare>>,
    commit_event: EventSender<Message<Commit>>,
}

impl<'a, T, U> ViewState<'a, T, U>
where
    T: Transport<ToReplica>,
    U: Transport<Reply>,
{
    fn new(
        replica: Arc<Replica>,
        view_num: u32,
        event: EventSender<ReplicaEvent>,
        source: &'a mut EventSource<ReplicaEvent>,
        transport: T,
        reply_transport: U,
    ) -> Self {
        let batch_state = if replica.is_primary(view_num) {
            let (event, source) = event_channel();
            let permits = Arc::new(Semaphore::new(1));
            Some(BatchState {
                event,
                session: tokio::spawn(batch_session(permits.clone(), source)),
                permits,
            })
        } else {
            None
        };
        Self {
            replica,
            view_num,
            event,
            source,
            transport,
            reply_transport,
            client_entires: Default::default(),
            batch_state,
            propose_op: Default::default(),
            prepare_op: Default::default(),
            commit_op: Default::default(),
            quorum_states: Default::default(),
            log_entries: Default::default(),
        }
    }

    async fn session(&mut self) -> crate::Result<()> {
        loop {
            assert!(self.commit_op <= self.prepare_op);
            // inefficient and anonying workaround for cannot exclusively borrow multiple values
            // maybe get solved by `get_many_mut` if it get stablized
            let mut prepare_session = None;
            let mut commit_session = None;
            for (&op, state) in &mut self.quorum_states {
                if op == self.prepare_op + 1 {
                    prepare_session = Some(&mut state.prepare_session)
                }
                if op == self.commit_op + 1 {
                    commit_session = Some(&mut state.commit_session)
                }
            }
            tokio::select! {
                event = self.source.next() => self.on_event(event?)?,
                requests = &mut self.batch_state.as_mut().unwrap().session,
                    if self.batch_state.is_some() => self.on_requests(requests??),
                entry = prepare_session.unwrap(),
                    if prepare_session.is_some() => self.on_prepare_quorum(entry??),
                commits = commit_session.unwrap(),
                    if commit_session.is_some() => self.on_commit_quorum(commits??),
            }
        }
    }

    fn on_event(&mut self, event: ReplicaEvent) -> crate::Result<()> {
        match event {
            ReplicaEvent::Message(ToReplica::Request(request)) => self.handle_request(request)?,
            ReplicaEvent::Message(ToReplica::PrePrepare(pre_prepare, requests)) => {
                self.handle_pre_prepare(pre_prepare.deserialize()?, requests)?
            }
            ReplicaEvent::Message(ToReplica::Prepare(prepare)) => {
                self.handle_prepare(prepare.deserialize()?)?
            }
            ReplicaEvent::Message(ToReplica::Commit(commit)) => {
                self.handle_commit(commit.deserialize()?)?
            }
            ReplicaEvent::ReplyReady(client_id, reply) => self.handle_reply_ready(client_id, reply),
        }
        Ok(())
    }

    fn handle_request(&mut self, request: Request) -> crate::Result<()> {
        match self.client_entires.get(&request.client_id) {
            Some(ClientEntry::Committing(request_num)) if *request_num >= request.request_num => {}
            Some(ClientEntry::Committed(reply)) if reply.request_num > request.request_num => {}
            _ => {
                if !self.replica.is_primary(self.view_num) {
                    todo!("relay Request to primary")
                } else {
                    self.client_entires.insert(
                        request.client_id,
                        ClientEntry::Committing(request.request_num),
                    );
                    self.batch_state.as_ref().unwrap().event.send(request)?
                }
            }
        }
        Ok(())
    }

    fn handle_pre_prepare(
        &mut self,
        pre_prepare: Message<PrePrepare>,
        requests: Vec<Request>,
    ) -> crate::Result<()> {
        self.get_or_insert_quorum_state(pre_prepare.op_num)
            .pre_prepare_event
            .as_ref()
            .expect("PrePrepare EventSender exists")
            .send((pre_prepare, requests))
    }

    fn handle_prepare(&mut self, prepare: Message<Prepare>) -> crate::Result<()> {
        self.get_or_insert_quorum_state(prepare.op_num)
            .prepare_event
            .send(prepare)
    }

    fn handle_commit(&mut self, commit: Message<Commit>) -> crate::Result<()> {
        self.get_or_insert_quorum_state(commit.op_num)
            .commit_event
            .send(commit)
    }

    fn get_or_insert_quorum_state(&mut self, op_num: u32) -> &mut QuorumState {
        assert!(!self.replica.is_primary(self.view_num));
        self.quorum_states.entry(op_num).or_insert_with(|| {
            let (pre_prepare_event, pre_prepare_source) = event_channel();
            let (prepare_event, prepare_source) = event_channel();
            let (commit_event, commit_source) = event_channel();
            let (commit_digest, digest) = promise_channel();
            QuorumState {
                prepare_session: tokio::spawn(backup_prepare_session(
                    self.replica.clone(),
                    self.view_num,
                    pre_prepare_source,
                    prepare_source,
                    commit_digest,
                    self.transport.clone(),
                )),
                commit_session: tokio::spawn(commit_session(
                    self.replica.clone(),
                    self.view_num,
                    op_num,
                    async move { Ok(digest.await?) },
                    commit_source,
                    self.transport.clone(),
                )),
                pre_prepare_event: Some(pre_prepare_event),
                prepare_event,
                commit_event,
            }
        })
    }

    fn handle_reply_ready(&mut self, client_id: u32, reply: Reply) {
        match self
            .client_entires
            .insert(client_id, ClientEntry::Committed(reply.clone()))
        {
            Some(ClientEntry::Committing(request_num)) if request_num == reply.request_num => self
                .replica
                .send_to_client(client_id, reply, self.reply_transport.clone()),
            Some(ClientEntry::Committing(request_num)) if request_num > reply.request_num => {}
            _ => unreachable!(),
        }
    }

    fn on_requests(&mut self, requests: Vec<Request>) {
        assert!(self.replica.is_primary(self.view_num));
        self.propose_op += 1;
        let (prepare_event, prepare_source) = event_channel();
        let (commit_event, commit_source) = event_channel();
        let (commit_digest, digest) = promise_channel();
        self.quorum_states.insert(
            self.propose_op,
            QuorumState {
                prepare_session: tokio::spawn(primary_prepare_session(
                    self.replica.clone(),
                    self.view_num,
                    self.propose_op,
                    requests,
                    prepare_source,
                    commit_digest,
                    self.transport.clone(),
                )),
                commit_session: tokio::spawn(commit_session(
                    self.replica.clone(),
                    self.view_num,
                    self.propose_op,
                    async move { Ok(digest.await?) },
                    commit_source,
                    self.transport.clone(),
                )),
                pre_prepare_event: None,
                prepare_event,
                commit_event,
            },
        );
    }

    fn on_prepare_quorum(&mut self, entry: LogEntry) {
        self.prepare_op += 1;
        for request in &entry.requests {
            if self.replica.is_primary(self.view_num) {
                assert_eq!(
                    self.client_entires.get(&request.client_id),
                    Some(&ClientEntry::Committing(request.request_num))
                )
            } else {
                self.client_entires.insert(
                    request.client_id,
                    ClientEntry::Committing(request.request_num),
                );
            }
        }
        self.log_entries.insert(self.prepare_op, entry);
    }

    fn on_commit_quorum(&mut self, commits: Quorum<Commit>) {
        self.commit_op += 1;
        let entry = self
            .log_entries
            .get_mut(&self.commit_op)
            .expect("committed log entry exists");
        entry.commits = commits;

        let replica_id = self.replica.id;
        let view_num = self.view_num;
        for ((request, app), event) in entry
            .requests
            .iter()
            .cloned()
            .zip(repeat(self.replica.app.clone()))
            .zip(repeat(self.event.clone()))
        {
            self.replica.spawner.spawn(async move {
                let reply = Reply {
                    request_num: request.request_num,
                    result: app.submit(request.op).await?,
                    replica_id,
                    view_num,
                };
                event.send(ReplicaEvent::ReplyReady(request.client_id, reply))
            });
        }

        if self.replica.is_primary(self.view_num) {
            self.batch_state.as_ref().unwrap().permits.add_permits(1)
        }
    }
}
