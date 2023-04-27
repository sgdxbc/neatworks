use larlis_core::{actor, app};

use crate::{Reply, Request};

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

impl actor::State<'_> for Replica {
    type Message = Request;

    fn update(&mut self, message: Self::Message) {
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

pub struct App<A>(pub A);

impl<A> app::SyncState<'_> for App<A>
where
    A: larlis_core::App + 'static,
{
    type Input = Upcall;
    type Output = (u32, Reply);

    fn update(&mut self, input: Self::Input) -> Self::Output {
        let result = self.0.update(input.op_num, &input.op);
        let message = Reply {
            request_num: input.request_num,
            result,
        };
        (input.client_id, message)
    }
}
