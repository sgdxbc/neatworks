use std::collections::HashMap;

use neat_core::{actor, app};

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

impl<U> actor::State<Request> for Replica<U>
where
    U: actor::State<Upcall>,
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

pub struct App<A> {
    pub state: A,
    replies: HashMap<u32, Reply>,
}

impl<A> From<A> for App<A> {
    fn from(value: A) -> Self {
        Self {
            state: value,
            replies: Default::default(),
        }
    }
}

impl<A> app::FunctionalState<Upcall> for App<A>
where
    A: neat_core::App + 'static,
{
    type Output<'a> = Option<(u32, Reply)>;

    fn update(&mut self, input: Upcall) -> Self::Output<'_> {
        match self.replies.get(&input.client_id) {
            Some(reply) if reply.request_num > input.request_num => return None,
            Some(reply) if reply.request_num == input.request_num => {
                return Some((input.client_id, reply.clone()))
            }
            _ => {}
        }
        let result = self.state.update(input.op_num, &input.op);
        let message = Reply {
            request_num: input.request_num,
            result,
        };
        self.replies.insert(input.client_id, message.clone());
        Some((input.client_id, message))
    }
}
