use std::{collections::HashMap, future::Future};

use derive_more::From;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::{
    spawn,
    sync::{
        mpsc::{unbounded_channel, UnboundedSender},
        oneshot,
    },
    task::JoinHandle,
};
use tokio_stream::wrappers::UnboundedReceiverStream;

use crate::{
    transport::{
        Source,
        Transmit::{ToAllReplica, ToClient},
    },
    App, SecretKey, Signature, Transport,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Request {
    client_id: u32,
    request_num: u32,
    op: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Reply {
    request_num: u32,
    result: Vec<u8>,
    replica_id: u8,
    view_num: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PrePrepare {
    view_num: u32,
    op_num: u32,
    digest: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Prepare {
    view_num: u32,
    op_num: u32,
    digest: Vec<u8>,
    replica_id: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Commit {
    view_num: u32,
    op_num: u32,
    digest: Vec<u8>,
    replica_id: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, From, Serialize, Deserialize)]
pub enum ToReplica {
    Request(Request),
    PrePrepare(PrePrepare, Signature, Vec<Request>),
    Prepare(Prepare, Signature),
    Commit(Commit, Signature),
}

#[derive(Debug)]
pub struct Client {
    id: u32,
    request_num: u32,
    num_faulty: usize,
}

impl Client {
    pub async fn invoke(
        &mut self,
        op: Vec<u8>,
        mut transport: impl Transport<ToReplica>,
        mut reply_stream: impl Source<Reply>,
    ) -> crate::Result<Vec<u8>> {
        self.request_num += 1;
        let request = Request {
            client_id: self.id,
            request_num: self.request_num,
            op,
        };
        transport
            .send(ToAllReplica(request.into()))
            .await
            .map_err(|_| "transport Request fail")?;
        let mut replies = HashMap::new();
        loop {
            let reply = reply_stream.next().await.ok_or("Reply source fail")?;
            if reply.request_num != self.request_num {
                continue;
            }
            replies.insert(reply.replica_id, reply.clone());
            let quorum_size = replies
                .values()
                .filter(|other_reply| other_reply.result == reply.result)
                .count();
            if quorum_size > self.num_faulty {
                break Ok(reply.result);
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct Replica {
    id: u8,
    secret_key: SecretKey,
    num_faulty: usize,
}

pub async fn replica_loop(
    replica: Replica,
    mut app: impl App,
    transport: impl Transport<ToReplica>,
    mut reply_transport: impl Transport<Reply>,
    message_source: impl Source<ToReplica>,
) -> crate::Result<()> {
    ViewLoop {
        replica: replica.clone(),
        view_num: 0,
        op_propose: 0,
        op_commit: 0,
        do_commits: Default::default(),
    }
    .run(&mut app, transport, &mut reply_transport, message_source)
    .await
}

struct LogEntry {
    requests: Vec<Request>,
    pre_prepare: (PrePrepare, Signature),
    prepares: HashMap<u8, (Prepare, Signature)>,
    commits: HashMap<u8, (Commit, Signature)>,
}

struct ViewLoop {
    replica: Replica,
    view_num: u32,
    op_propose: u32,
    op_commit: u32,
    do_commits: HashMap<u32, DoCommitHandle>,
}

struct DoCommitHandle {
    handle: JoinHandle<crate::Result<LogEntry>>,
    pre_prepare: Option<oneshot::Sender<(PrePrepare, Signature, Vec<Request>)>>,
    prepare: UnboundedSender<(Prepare, Signature)>,
    commit: UnboundedSender<(Commit, Signature)>,
}

impl ViewLoop {
    async fn run(
        &mut self,
        app: &mut impl App,
        transport: impl Transport<ToReplica>,
        reply_transport: &mut impl Transport<Reply>,
        mut message_source: impl Source<ToReplica>,
    ) -> crate::Result<()> {
        let (request_tx, request_rx) = unbounded_channel();
        let (entry_tx, entry_rx) = unbounded_channel();
        let mut request_source = UnboundedReceiverStream::new(request_rx);
        let mut entry_source = UnboundedReceiverStream::new(entry_rx);
        let mut request_loop = RequestLoop {
            replica: self.replica.clone(),
            replies: Default::default(),
            requests: Default::default(),
        };
        loop {
            let do_commit = self.do_commits.get_mut(&(self.op_commit + 1));
            tokio::select! {
                message = message_source.next() => {
                    self.handle_message(message.ok_or("message stream fail")?, transport.clone())?
                }
                entry = &mut do_commit.unwrap().handle, if do_commit.is_some() => {
                    self.handle_commit();
                    entry_tx.send(entry??)?
                }
                requests = request_loop.run(
                    app,
                    reply_transport,
                    &mut request_source,
                    &mut entry_source,
                ) => {}
            }
        }
    }

    fn handle_message(
        &mut self,
        message: ToReplica,
        transport: impl Transport<ToReplica>,
    ) -> crate::Result<()> {
        match message {
            ToReplica::Request(_) => unreachable!(),
            ToReplica::PrePrepare(pre_prepare, signature, requests) => self
                .get_or_insert_sender(pre_prepare.op_num, transport.clone())
                .pre_prepare
                .take()
                .map(|tx| {
                    tx.send((pre_prepare, signature, requests))
                        .map_err(|_| "PrePrepare send fail")
                })
                .unwrap_or(Ok(()))?,
            ToReplica::Prepare(prepare, signature) => self
                .get_or_insert_sender(prepare.op_num, transport.clone())
                .prepare
                .send((prepare, signature))?,
            ToReplica::Commit(commit, signature) => self
                .get_or_insert_sender(commit.op_num, transport.clone())
                .commit
                .send((commit, signature))?,
        }
        Ok(())
    }

    fn get_or_insert_sender(
        &mut self,
        op_num: u32,
        transport: impl Transport<ToReplica>,
    ) -> &mut DoCommitHandle {
        self.do_commits.entry(op_num).or_insert_with(|| {
            let (pre_prepare_tx, pre_prepare_rx) = oneshot::channel();
            let (prepare_tx, prepare_rx) = unbounded_channel();
            let (commit_tx, commit_rx) = unbounded_channel();
            let handle = spawn(do_commit(
                self.replica.clone(),
                self.view_num,
                transport,
                async {
                    let pre_prepare = pre_prepare_rx.await?;
                    Ok(pre_prepare)
                },
                UnboundedReceiverStream::new(prepare_rx),
                UnboundedReceiverStream::new(commit_rx),
            ));
            DoCommitHandle {
                handle,
                pre_prepare: Some(pre_prepare_tx),
                prepare: prepare_tx,
                commit: commit_tx,
            }
        })
    }

    fn handle_commit(&mut self) {
        self.do_commits.remove(&self.op_commit).unwrap();
    }
}

struct RequestLoop {
    replica: Replica,
    replies: HashMap<u32, Reply>,
    requests: Vec<Request>,
}

impl RequestLoop {
    async fn run(
        &mut self,
        app: &mut impl App,
        reply_transport: &mut impl Transport<Reply>,
        request_source: &mut impl Source<Request>,
        entry_source: &mut impl Source<LogEntry>,
    ) -> crate::Result<()> {
        loop {
            tokio::select! {
                request = request_source.next() => {
                    self.handle_request(request.ok_or("request source fail")?, reply_transport).await?
                }
                entry = entry_source.next() => {
                    self.handle_entry(entry.ok_or("entry source fail")?, reply_transport, app).await?
                }
            }
        }
    }

    async fn handle_request(
        &mut self,
        request: Request,
        reply_transport: &mut impl Transport<Reply>,
    ) -> crate::Result<()> {
        match self.replies.get(&request.client_id) {
            Some(reply) if reply.request_num > request.request_num => {}
            Some(reply) if reply.request_num == request.request_num => reply_transport
                .send(ToClient(request.client_id, reply.clone()))
                .await
                .map_err(|_| "transport Reply fail")?,
            _ => self.requests.push(request),
        }
        Ok(())
    }

    async fn handle_entry(
        &mut self,
        entry: LogEntry,
        reply_transport: &mut impl Transport<Reply>,
        app: &mut impl App,
    ) -> crate::Result<()> {
        let view_num = entry.pre_prepare.0.view_num;
        for request in entry.requests {
            let result = app.execute(&request.op).await;
            let reply = Reply {
                request_num: request.request_num,
                result,
                replica_id: self.replica.id,
                view_num,
            };
            let evicted = self.replies.insert(request.client_id, reply.clone());
            if let Some(evicted) = evicted {
                assert!(evicted.request_num < reply.request_num);
            }
            reply_transport
                .send(ToClient(request.client_id, reply))
                .await
                .map_err(|_| "transport Reply fail")?
        }
        Ok(())
    }
}

async fn do_commit(
    replica: Replica,
    view_num: u32,
    mut transport: impl Transport<ToReplica>,
    pre_prepare: impl Future<Output = crate::Result<(PrePrepare, Signature, Vec<Request>)>> + Send,
    mut prepare_source: impl Source<(Prepare, Signature)>,
    mut commit_source: impl Source<(Commit, Signature)>,
) -> crate::Result<LogEntry> {
    let pre_prepare = pre_prepare.await?;
    assert_eq!(pre_prepare.0.view_num, view_num);
    let op_num = pre_prepare.0.op_num;
    let digest = pre_prepare.0.digest.clone();
    let prepare = Prepare {
        view_num,
        op_num,
        digest: digest.clone(),
        replica_id: replica.id,
    };
    let signature = crate::signature(&prepare, &replica.secret_key);
    transport
        .send(ToAllReplica((prepare.clone(), signature).into()))
        .await
        .map_err(|_| "transport preprare fail")?;
    let mut prepares = HashMap::new();
    prepares.insert(replica.id, (prepare, signature));
    while prepares.len() < 2 * replica.num_faulty + 1 {
        let (prepare, signature) = prepare_source.next().await.ok_or("prepare stream fail")?;
        assert_eq!(prepare.op_num, op_num);
        if prepare.view_num == view_num && prepare.digest == digest {
            prepares.insert(prepare.replica_id, (prepare, signature));
        }
    }
    let commit = Commit {
        view_num,
        op_num,
        digest: digest.clone(),
        replica_id: replica.id,
    };
    let signature = crate::signature(&commit, &replica.secret_key);
    transport
        .send(ToAllReplica((commit.clone(), signature).into()))
        .await
        .map_err(|_| "transport commit fail")?;
    let mut commits = HashMap::new();
    commits.insert(replica.id, (commit, signature));
    while commits.len() < 2 * replica.num_faulty + 1 {
        let (commit, signature) = commit_source.next().await.ok_or("commit stream fail")?;
        assert_eq!(commit.op_num, op_num);
        if commit.view_num == view_num && commit.digest == digest {
            commits.insert(commit.replica_id, (commit, signature));
        }
    }
    Ok(LogEntry {
        requests: pre_prepare.2,
        pre_prepare: (pre_prepare.0, pre_prepare.1),
        prepares,
        commits,
    })
}
