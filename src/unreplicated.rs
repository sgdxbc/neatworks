use std::{collections::HashMap, sync::Arc};

use borsh::{BorshDeserialize, BorshSerialize};
use derive_more::From;
use tokio::time::{timeout_at, Instant};

use crate::{
    model::{EventSender, EventSource, Transport},
    submit, Client, Replica,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash, BorshSerialize, BorshDeserialize)]
pub struct Request {
    client_id: u32,
    request_num: u32,
    op: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, BorshSerialize, BorshDeserialize)]
pub struct Reply {
    request_num: u32,
    result: Vec<u8>,
}

pub async fn client_loop(
    client: Arc<Client>,
    mut receiver: submit::Receiver<Vec<u8>, Vec<u8>>,
    mut source: EventSource<Reply>,
    transport: impl Transport<Request>,
) -> crate::Result<()> {
    let mut request_num = 0;

    while let Some((op, result_sender)) = receiver.option_next().await {
        request_num += 1;
        let request = Request {
            client_id: client.id,
            request_num,
            op,
        };

        let result = 'retry: loop {
            transport
                .send_to(client.addr_book.replica_addr(0)?, request.clone())
                .await?;
            let deadline = Instant::now() + client.retry_interval;
            loop {
                let reply = match timeout_at(deadline, source.next()).await {
                    Ok(reply) => reply?,
                    Err(_) => continue 'retry,
                };
                assert!(reply.request_num <= request_num);
                if reply.request_num == request_num {
                    break 'retry reply.result;
                }
            }
        };
        result_sender
            .send(result)
            .map_err(|_| crate::err!("unexpected result channel closing"))?
    }
    Ok(())
}

#[derive(Debug, From)]
pub enum ReplicaEvent {
    Message(Request),
    ReplyReady(u32, Reply),
}

enum ClientEntry {
    Submitted(u32),
    Replied(Reply),
}

pub async fn replica_loop(
    replica: Arc<Replica>,
    event: EventSender<ReplicaEvent>,
    mut source: EventSource<ReplicaEvent>,
    reply_transport: impl Transport<Reply>,
) -> crate::Result<()> {
    let mut entries = HashMap::<_, ClientEntry>::new();

    loop {
        match source.next().await? {
            ReplicaEvent::Message(request) => {
                match entries.get(&request.client_id) {
                    Some(ClientEntry::Submitted(request_num))
                        if *request_num >= request.request_num =>
                    {
                        continue
                    }
                    Some(ClientEntry::Replied(reply))
                        if reply.request_num > request.request_num =>
                    {
                        continue
                    }
                    Some(ClientEntry::Replied(reply))
                        if reply.request_num == request.request_num =>
                    {
                        let reply_transport = reply_transport.clone();
                        let addr = replica.addr_book.client_addr(request.client_id)?;
                        let reply = reply.clone();
                        replica
                            .spawner
                            .spawn(async move { reply_transport.send_to(addr, reply).await });
                        continue;
                    }
                    _ => {}
                }
                entries.insert(
                    request.client_id,
                    ClientEntry::Submitted(request.request_num),
                );
                let event = event.clone();
                let app = replica.app.clone();
                replica.spawner.spawn(async move {
                    let reply = Reply {
                        request_num: request.request_num,
                        result: app.submit(request.op).await?,
                    };
                    event.send(ReplicaEvent::ReplyReady(request.client_id, reply))
                })
            }
            ReplicaEvent::ReplyReady(client_id, reply) => {
                entries.insert(client_id, ClientEntry::Replied(reply.clone()));
                let reply_transport = reply_transport.clone();
                let addr = replica.addr_book.client_addr(client_id)?;
                replica
                    .spawner
                    .spawn(async move { reply_transport.send_to(addr, reply).await });
            }
        }
    }
}
