use std::collections::HashMap;

use neat_core::{actor::State, message::Lift};

use crate::{Reply, Request};

pub struct Upcall {
    op_num: u32,
    client_id: u32,
    request_num: u32,
    op: Vec<u8>,
}

pub struct Replica<U> {
    op_num: u32,
    pub upcall: U,
}

impl<U> Replica<U> {
    pub fn new(upcall: U) -> Self {
        Self { op_num: 0, upcall }
    }
}

impl<U> State<Request> for Replica<U>
where
    U: State<Upcall>,
{
    fn update(&mut self, message: Request) {
        self.op_num += 1;
        let upcall = Upcall {
            op_num: self.op_num,
            client_id: message.client_id,
            request_num: message.request_num,
            op: message.op,
        };
        self.upcall.update(upcall);
    }
}

#[derive(Debug, Default)]
pub struct AppLift {
    replies: HashMap<u32, Reply>,
}

impl<A> Lift<A, Upcall> for AppLift
where
    A: neat_core::App + 'static,
{
    type Out<'a> = Option<(u32, Reply)>;

    fn update<'a>(&'a mut self, state: &'a mut A, message: Upcall) -> Self::Out<'a> {
        match self.replies.get(&message.client_id) {
            Some(reply) if reply.request_num > message.request_num => return None,
            Some(reply) if reply.request_num == message.request_num => {
                return Some((message.client_id, reply.clone()))
            }
            _ => {}
        }
        let result = state.update(message.op_num, &message.op);
        let reply = Reply {
            request_num: message.request_num,
            result,
        };
        self.replies.insert(message.client_id, reply.clone());
        Some((message.client_id, reply))
    }
}
