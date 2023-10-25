use std::{collections::HashMap, time::Duration};

use serde::{Deserialize, Serialize};

use crate::context::{Addr, MultiplexReceive};

#[derive(Debug)]
pub struct CloseLoop<W> {
    workloads: HashMap<Addr, W>,
    pub latencies: Vec<Duration>,
}

pub trait Workload {
    fn submit(&mut self);
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Message<M> {
    Workload(M),
    Complete(Duration),
}

impl<W> CloseLoop<W> {
    pub fn bootstrap(&mut self)
    where
        W: Workload,
    {
        for workload in self.workloads.values_mut() {
            workload.submit()
        }
    }
}

impl<W> MultiplexReceive for CloseLoop<W>
where
    W: MultiplexReceive + Workload,
{
    type Message = Message<W::Message>;

    fn handle(&mut self, receiver: Addr, remote: Addr, message: Self::Message) {
        match message {
            Message::Workload(message) => self
                .workloads
                .get_mut(&receiver)
                .unwrap()
                .handle(receiver, remote, message),
            Message::Complete(_) => unimplemented!(),
        }
    }

    fn handle_loopback(&mut self, receiver: Addr, message: Self::Message) {
        match message {
            Message::Workload(message) => self
                .workloads
                .get_mut(&receiver)
                .unwrap()
                .handle_loopback(receiver, message),
            Message::Complete(latency) => {
                self.latencies.push(latency);
                self.workloads.get_mut(&receiver).unwrap().submit()
            }
        }
    }

    fn on_timer(&mut self, receiver: Addr, id: crate::context::TimerId) {
        self.workloads
            .get_mut(&receiver)
            .unwrap()
            .on_timer(receiver, id)
    }

    fn on_pace(&mut self) {
        for workload in self.workloads.values_mut() {
            workload.on_pace()
        }
    }
}
