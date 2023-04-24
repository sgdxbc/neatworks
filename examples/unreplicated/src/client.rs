use larlis_core::actor;

use crate::message::{Reply, Request};

enum Message {
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
