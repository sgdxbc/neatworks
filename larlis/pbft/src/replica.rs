use larlis_core::{actor, app};
use serde::{Serialize, Deserialize};

use crate::{Reply, Request};

pub struct Upcall {
    view_num: u32,
    op_num: u32,
    client_id: u32,
    request_num: u32,
    op: Vec<u8>,
    replica_id: u8,
}

pub struct App<A>(pub A);

impl<A> app::PureState<'_> for App<A>
where
    A: larlis_core::App + 'static,
{
    type Input = Upcall;
    type Output<'a> = (u32, Reply);

    fn update(&mut self, input: Self::Input) -> Self::Output<'_> {
        let result = self.0.update(input.op_num, &input.op);
        let message = Reply {
            request_num: input.request_num,
            result,
            replica_id: input.replica_id,
            view_num: input.view_num,
        };
        (input.client_id, message)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToReplica {
    Request(Request),
    PrePrepare(PrePrepare, Vec<u8>),
    Prepare(Prepare),
    Commit(Commit),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrePrepare {
    view_num: u32,
    op_num: u32,
    digest: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prepare {
    view_num: u32,
    op_num: u32,
    digest: [u8; 32],
    replica_id: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Commit {
    view_num: u32,
    op_num: u32,
    digest: [u8; 32],
    replica_id: u8,
}


pub struct Replica<U> {
    id: u8,
    view_num: u32,
    op_num: u32,
    pub upcall: U,
}

impl<U> Replica<U> {
    pub fn new(id: u8, upcall: U) -> Self {
        Self {
            id,
            view_num: 0,
            op_num: 0,
            upcall,
        }
    }
}

impl<U> actor::State<'_> for Replica<U>
where
    U: for<'m> actor::State<'m, Message = Upcall>,
{
    type Message = Request;

    fn update(&mut self, message: Self::Message) {
        self.op_num += 1;
        let upcall = Upcall {
            replica_id: self.id,
            view_num: self.view_num,
            op_num: self.op_num,
            client_id: message.client_id,
            request_num: message.request_num,
            op: message.op,
        };
        self.upcall.update(upcall);
    }
}

impl<U> Replica<U> {
    fn handle_request(&mut self, message: Request) {
        //
    }
}