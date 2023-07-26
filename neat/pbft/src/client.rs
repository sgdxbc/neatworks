use std::collections::HashMap;

use neat_core::actor::State;

use crate::{Reply, Request};

pub enum Message {
    Handle(Reply),
    Tick,
    Invoke(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Result(pub Vec<u8>);

pub struct Client<O, R> {
    id: u32,
    f: usize,
    request_num: u32,
    ticked: bool,
    op: Option<Vec<u8>>,
    replies: HashMap<u8, Reply>,
    pub egress: O,
    pub result: R,
}

impl<O, R> Client<O, R> {
    pub fn new(id: u32, f: usize, egress: O, result: R) -> Self {
        Self {
            id,
            f,
            request_num: 0,
            ticked: false,
            op: None,
            replies: Default::default(),
            egress,
            result,
        }
    }
}

impl<O, R> State<Message> for Client<O, R>
where
    O: State<Request>,
    R: State<Result>,
{
    fn update(&mut self, message: Message) {
        match message {
            Message::Invoke(op) => self.invoke(op),
            Message::Handle(message) => self.handle(message),
            Message::Tick => self.tick(),
        }
    }
}

impl<O, R> Client<O, R>
where
    O: State<Request>,
    R: State<Result>,
{
    fn invoke(&mut self, op: Vec<u8>) {
        assert!(self.op.is_none());
        self.request_num += 1;
        self.op = Some(op);
        self.replies.clear();
        self.ticked = false;
        self.do_request();
    }

    fn do_request(&mut self) {
        let message = Request {
            client_id: self.id,
            request_num: self.request_num,
            op: self.op.clone().unwrap(),
        };
        self.egress.update(message);
    }

    fn handle(&mut self, message: Reply) {
        if self.op.is_none() || message.request_num != self.request_num {
            return;
        }

        self.replies.insert(message.replica_id, message);
        let mut replies = Vec::from_iter(self.replies.values());
        #[allow(clippy::int_plus_one)] // to match common description of the protocol
        while replies.len() >= self.f + 1 {
            let reply = replies.pop().unwrap();
            let matched;
            (matched, replies) = replies
                .into_iter()
                .partition(|r| (&r.result, r.view_num) == (&reply.result, reply.view_num));
            if matched.len() >= self.f {
                self.op = None;
                self.result.update(Result(reply.result.clone()));
                // TODO update cached view number
                break;
            }
        }
    }

    fn tick(&mut self) {
        if self.op.is_none() {
            return;
        }
        if !self.ticked {
            self.ticked = true;
            return;
        }
        // self.do_request();
        panic!("request timeout")
    }
}
