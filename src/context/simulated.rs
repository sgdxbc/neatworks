use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
    time::Duration,
};

use super::{
    crypto::{Sign, Signer},
    Host, Receivers, To,
};

#[derive(Debug, Clone)]
enum Event<M> {
    Message(Host, Host, M),
    LoopbackMessage(Host, M),
    Timer(Host),
}

pub type TimerId = u32;

#[derive(Debug)]
pub struct Context<M> {
    pub num_faulty: usize,
    pub num_replica: usize,
    source: Host,
    timeline: Arc<Mutex<Timeline<M>>>,
}

#[derive(Debug)]
struct Timeline<M> {
    now: Duration,
    id: u32,
    events: BTreeMap<(Duration, u32), Event<M>>,
    timers: HashMap<TimerId, Timer>,
}

#[derive(Debug)]
struct Timer {
    duration: Duration,
    key: (Duration, u32),
}

impl<M> Timeline<M> {
    fn add_event(&mut self, offset: Duration, event: Event<M>) {
        self.id += 1;
        let evicted = self.events.insert((self.now + offset, self.id), event);
        assert!(evicted.is_none())
    }

    fn add_timer_event(&mut self, offset: Duration, event: Event<M>) {
        self.add_event(offset, event);
        let evicted = self.timers.insert(
            self.id,
            Timer {
                duration: offset,
                key: (self.now + offset, self.id),
            },
        );
        assert!(evicted.is_none())
    }
}

impl<M> Context<M> {
    pub fn send<N>(&mut self, to: To, message: N)
    where
        M: Sign<N> + Clone,
    {
        let message = M::sign(message, &Signer::Simulated);
        let mut timeline = self.timeline.try_lock().unwrap();
        if matches!(to, To::Loopback | To::AllReplicaWithLoopback) {
            timeline.add_event(
                Duration::ZERO,
                Event::LoopbackMessage(self.source, message.clone()),
            )
        }
        match to {
            To::Host(host) => {
                timeline.add_event(Duration::ZERO, Event::Message(host, self.source, message))
            }
            To::Hosts(hosts) => {
                for host in hosts {
                    assert_ne!(host, self.source);
                    timeline.add_event(
                        Duration::ZERO,
                        Event::Message(host, self.source, message.clone()),
                    )
                }
            }
            To::AllReplica | To::AllReplicaWithLoopback => {
                for index in 0..self.num_replica {
                    if Host::Replica(index as _) != self.source {
                        timeline.add_event(
                            Duration::ZERO,
                            Event::Message(Host::Replica(index as _), self.source, message.clone()),
                        )
                    }
                }
            }
            To::Loopback => {}
        }
    }

    pub fn set(&self, duration: Duration) -> TimerId {
        let mut timeline = self.timeline.try_lock().unwrap();
        timeline.add_timer_event(duration, Event::Timer(self.source));
        timeline.id
    }

    pub fn unset(&self, id: TimerId) {
        let mut timeline = self.timeline.try_lock().unwrap();
        let timer = timeline.timers.remove(&id).unwrap();
        timeline.events.remove(&timer.key).unwrap();
    }
}

#[derive(Debug)]
pub struct Dispatch<M> {
    num_faulty: usize,
    num_replica: usize,
    timeline: Arc<Mutex<Timeline<M>>>,
}

impl<M> Dispatch<M> {
    pub fn register(&self, receiver: Host) -> crate::Context<M> {
        crate::Context::Simulated(Context {
            num_faulty: self.num_faulty,
            num_replica: self.num_replica,
            source: receiver,
            timeline: self.timeline.clone(),
        })
    }

    pub fn deliver_event(&self, receivers: &mut impl Receivers<Message = M>) -> bool {
        let mut timeline = self.timeline.lock().unwrap();
        let Some(((now, id), event)) = timeline.events.pop_first() else {
            return false;
        };
        assert!(now >= timeline.now);
        timeline.now = now;
        match event {
            Event::Message(receiver, remote, message) => {
                receivers.handle(receiver, remote, message)
            }
            Event::LoopbackMessage(receiver, message) => {
                receivers.handle_loopback(receiver, message)
            }
            Event::Timer(receiver) => {
                let offset = timeline.timers[&id].duration;
                timeline.add_timer_event(offset, Event::Timer(receiver));
                receivers.on_timer(receiver, crate::context::TimerId::Simulated(id))
            }
        }
        true
    }
}
