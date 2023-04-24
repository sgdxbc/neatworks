use larlis_core::actor;

use crate::message::{Reply, Request};

pub enum Message {
    Handle(Reply),
    Tick,
    Invoke(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Result(Vec<u8>);

pub struct Client {
    id: u32,
    request_num: u32,
    ticked: bool,
    op: Option<Vec<u8>>,
    outgress: actor::Effect<Request>,
    result: actor::Effect<Result>,
}

impl Client {
    pub fn new(id: u32, outgress: actor::Effect<Request>, result: actor::Effect<Result>) -> Self {
        Self {
            id,
            request_num: 0,
            ticked: false,
            op: None,
            outgress,
            result,
        }
    }
}

impl actor::State for Client {
    type Message<'a> = Message;

    fn update(&mut self, message: Self::Message<'_>) {
        match message {
            Message::Invoke(op) => self.invoke(op),
            Message::Handle(message) => self.handle(message),
            Message::Tick => self.tick(),
        }
    }
}

impl Client {
    fn invoke(&mut self, op: Vec<u8>) {
        assert!(self.op.is_none());
        self.request_num += 1;
        self.op = Some(op);
        self.ticked = false;
        self.do_request();
    }

    fn do_request(&mut self) {
        let message = Request {
            client_id: self.id,
            request_num: self.request_num,
            op: self.op.clone().unwrap(),
        };
        self.outgress.update(message);
    }

    fn handle(&mut self, message: Reply) {
        if self.op.is_none() || message.request_num != self.request_num {
            return;
        }
        self.op = None;
        self.result.update(Result(message.result));
    }

    fn tick(&mut self) {
        if self.op.is_none() {
            return;
        }
        if !self.ticked {
            self.ticked = true;
            return;
        }
        //
        self.do_request();
    }
}
