use std::{collections::HashMap, error::Error, pin::pin};

use futures_util::{Sink, SinkExt, Stream, StreamExt};
use serde::{Deserialize, Serialize};

use crate::App;

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
}

#[derive(Debug)]
pub struct Client {
    id: u32,
    request_num: u32,
}

impl Client {
    pub async fn invoke(
        &mut self,
        op: Vec<u8>,
        tx: impl Sink<Request>,
        rx: impl Stream<Item = Reply>,
    ) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>> {
        let mut tx = pin!(tx);
        self.request_num += 1;
        let request = Request {
            client_id: self.id,
            request_num: self.request_num,
            op,
        };
        tx.send(request).await.map_err(|_| "tx failure")?;

        let mut rx = pin!(rx);
        loop {
            let reply = rx.next().await.ok_or("rx failure")?;
            if reply.request_num == self.request_num {
                return Ok(reply.result);
            }
        }
    }
}

pub async fn replica_loop(
    mut app: impl App,
    tx: impl Sink<Reply>,
    rx: impl Stream<Item = Request>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let mut tx = pin!(tx);
    let mut rx = pin!(rx);
    let mut replies = HashMap::<_, Reply>::new();
    loop {
        let request = rx.next().await.ok_or("rx failure")?;
        match replies.get(&request.client_id) {
            Some(reply) if reply.request_num > request.request_num => continue,
            Some(reply) if reply.request_num == request.request_num => {
                tx.send(reply.clone()).await.map_err(|_| "tx failure")?;
                continue;
            }
            _ => {}
        }
        let reply = Reply {
            request_num: request.request_num,
            result: app.execute(&request.op),
        };
        replies.insert(request.client_id, reply.clone());
        tx.send(reply).await.map_err(|_| "tx failure")?
    }
}
