use std::{collections::HashMap, sync::Mutex, time::Duration};

use serde::{Deserialize, Serialize};

use crate::{
    client::OnResult,
    common::Timer,
    context::{
        crypto::{DigestHash, Sign, Signed, Verify},
        ClientIndex, Context, Host, Receivers, To,
    },
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Message {
    Request(Signed<Request>),
    Reply(Signed<Reply>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    client_index: ClientIndex,
    request_num: u32,
    op: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reply {
    request_num: u32,
    result: Vec<u8>,
}

pub struct Client {
    index: ClientIndex,
    shared: Mutex<ClientShared>,
}

struct ClientShared {
    context: Context<Message>,
    request_num: u32,
    op: Option<Vec<u8>>,
    on_result: Option<Box<dyn OnResult + Send + Sync>>,
    resend_timer: Timer,
}

impl Client {
    pub fn new(context: Context<Message>, index: ClientIndex) -> Self {
        Self {
            index,
            shared: Mutex::new(ClientShared {
                context,
                request_num: 0,
                op: None,
                on_result: None,
                resend_timer: Timer::new(Duration::from_millis(100)),
            }),
        }
    }
}

impl crate::Client for Client {
    type Message = Message;

    fn invoke(&self, op: Vec<u8>, on_result: impl Into<Box<dyn OnResult + Send + Sync>>) {
        let shared = &mut *self.shared.lock().unwrap();
        shared.request_num += 1;
        assert!(shared.op.is_none());
        shared.op = Some(op.clone());
        shared.on_result = Some(on_result.into());
        shared.resend_timer.set(&mut shared.context);

        let request = Request {
            client_index: self.index,
            request_num: shared.request_num,
            op,
        };
        shared.context.send(To::replica(0), request)
    }

    fn handle(&self, message: Self::Message) {
        let Message::Reply(reply) = message else {
            unimplemented!()
        };
        let shared = &mut *self.shared.lock().unwrap();
        if reply.inner.request_num != shared.request_num {
            return;
        }
        shared.op.take().unwrap();
        shared.resend_timer.unset(&mut shared.context);
        shared.on_result.take().unwrap().apply(reply.inner.result);
    }
}

pub struct Replica {
    context: Context<Message>,
    replies: HashMap<ClientIndex, Reply>,
    app: (),
}

impl Replica {
    pub fn new(context: Context<Message>) -> Self {
        Self {
            context,
            replies: Default::default(),
            app: (),
        }
    }
}

impl Receivers for Replica {
    type Message = Message;

    fn handle(&mut self, receiver: Host, remote: Host, message: Self::Message) {
        assert_eq!(receiver, Host::Replica(0));
        let Message::Request(request) = message else {
            unimplemented!()
        };
        let Host::Client(index) = remote else {
            unimplemented!()
        };
        match self.replies.get(&index) {
            Some(reply) if reply.request_num > request.inner.request_num => return,
            Some(reply) if reply.request_num == request.inner.request_num => {
                self.context.send(To::Host(remote), reply.clone());
                return;
            }
            _ => {}
        }
        let reply = Reply {
            request_num: request.inner.request_num,
            result: Default::default(), // TODO
        };
        self.replies.insert(index, reply.clone());
        self.context.send(To::Host(remote), reply)
    }

    fn handle_loopback(&mut self, _: Host, _: Self::Message) {
        unreachable!()
    }

    fn on_timer(&mut self, _: Host, _: crate::context::TimerId) {
        unreachable!()
    }
}

impl DigestHash for Request {
    fn hash(&self, hasher: &mut crate::context::crypto::Hasher) {
        hasher.update(self.client_index.to_le_bytes());
        hasher.update(self.request_num.to_le_bytes());
        hasher.update(&self.op)
    }
}

impl DigestHash for Reply {
    fn hash(&self, hasher: &mut crate::context::crypto::Hasher) {
        hasher.update(self.request_num.to_le_bytes());
        hasher.update(&self.result)
    }
}

impl Sign<Request> for Message {
    fn sign(message: Request, signer: &crate::context::crypto::Signer) -> Self {
        Self::Request(signer.sign_private(message))
    }
}

impl Sign<Reply> for Message {
    fn sign(message: Reply, signer: &crate::context::crypto::Signer) -> Self {
        Self::Reply(signer.sign_private(message))
    }
}

impl Verify for Message {
    fn verify(
        &self,
        verifier: &crate::context::crypto::Verifier,
    ) -> Result<(), crate::context::crypto::Invalid> {
        match self {
            Self::Request(message) => verifier.verify(message, None),
            Self::Reply(message) => verifier.verify(message, 0),
        }
    }
}