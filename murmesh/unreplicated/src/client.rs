use murmesh_core::actor;

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
    request_num: u32,
    ticked: bool,
    op: Option<Vec<u8>>,
    pub egress: O,
    pub result: R,
}

impl<O, R> Client<O, R> {
    pub fn new(id: u32, egress: O, result: R) -> Self {
        Self {
            id,
            request_num: 0,
            ticked: false,
            op: None,
            egress,
            result,
        }
    }
}

impl<O, R> actor::State<'_> for Client<O, R>
where
    O: for<'m> actor::State<'m, Message = Request>,
    R: for<'m> actor::State<'m, Message = Result>,
{
    type Message = Message;

    fn update(&mut self, message: Self::Message) {
        match message {
            Message::Invoke(op) => self.invoke(op),
            Message::Handle(message) => self.handle(message),
            Message::Tick => self.tick(),
        }
    }
}

impl<O, R> Client<O, R>
where
    O: for<'m> actor::State<'m, Message = Request>,
    R: for<'m> actor::State<'m, Message = Result>,
{
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
        self.egress.update(message);
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
