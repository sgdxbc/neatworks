use larlis_core::actor;

use crate::message::{Reply, Request};

pub struct Upcall {
    op_num: u32,
    client_id: u32,
    request_num: u32,
    op: Vec<u8>,
}

pub struct Replica {
    op_num: u32,
    upcall: actor::Effect<Upcall>,
}

impl Replica {
    pub fn new(upcall: actor::Effect<Upcall>) -> Self {
        Self { op_num: 0, upcall }
    }
}

impl actor::State for Replica {
    type Message<'a> = Request;

    fn update(&mut self, message: Self::Message<'_>) {
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

pub struct App<A>(pub A, pub actor::Effect<(u32, Reply)>);

impl<A> actor::State for App<A>
where
    A: larlis_core::App,
{
    type Message<'a> = Upcall;

    fn update(&mut self, message: Self::Message<'_>) {
        let result = self.0.execute(message.op_num, &message.op);
        let client_id = message.client_id;
        let message = Reply {
            request_num: message.request_num,
            result,
        };
        self.1.update((client_id, message));
    }
}
