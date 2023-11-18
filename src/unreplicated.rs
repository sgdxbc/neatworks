use std::collections::HashMap;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};

use crate::{
    transport::{
        Source,
        Transmit::{ToClient, ToReplica},
    },
    App, Transport,
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
        mut transport: impl Transport<Request>,
        mut reply_source: impl Source<Reply>,
    ) -> crate::Result<Vec<u8>> {
        self.request_num += 1;
        let request = Request {
            client_id: self.id,
            request_num: self.request_num,
            op,
        };
        transport
            .send(ToReplica(0, request))
            .await
            .map_err(|_| "tranport Request fail")?;
        loop {
            let reply = reply_source.next().await.ok_or("Reply source fail")?;
            if reply.request_num == self.request_num {
                break Ok(reply.result);
            }
        }
    }
}

pub async fn replica_loop(
    mut app: impl App,
    mut transport: impl Transport<Reply>,
    mut request_source: impl Source<Request>,
) -> crate::Result<()> {
    let mut replies = HashMap::<_, Reply>::new();
    loop {
        let request = request_source.next().await.ok_or("Request source fail")?;
        match replies.get(&request.client_id) {
            Some(reply) if reply.request_num > request.request_num => continue,
            Some(reply) if reply.request_num == request.request_num => {
                transport
                    .send(ToClient(request.client_id, reply.clone()))
                    .await
                    .map_err(|_| "transport Reply fail")?;
                continue;
            }
            _ => {}
        }
        let reply = Reply {
            request_num: request.request_num,
            result: app.execute(&request.op).await,
        };
        replies.insert(request.client_id, reply.clone());
        transport
            .send(ToClient(request.client_id, reply))
            .await
            .map_err(|_| "transport Reply fail")?
    }
}
