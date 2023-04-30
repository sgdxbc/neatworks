use larlis_core::{actor, app};

use crate::{Reply, Request};

pub struct Upcall {
    op_num: u32,
    client_id: u32,
    request_num: u32,
    op: Vec<u8>,
}

pub struct Replica<U> {
    op_num: u32,
    upcall: U,
}

impl<U> Replica<U> {
    pub fn new(upcall: U) -> Self {
        Self { op_num: 0, upcall }
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
            op_num: self.op_num,
            client_id: message.client_id,
            request_num: message.request_num,
            op: message.op,
        };
        self.upcall.update(upcall);
    }
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
        };
        (input.client_id, message)
    }
}
