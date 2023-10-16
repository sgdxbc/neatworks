//! A context based on tokio and asynchronous IO.
//!
//! Although supported by an asynchronous reactor, protocol code, i.e.,
//! `impl Receivers` is still synchronous and running in a separated thread.

use std::{collections::HashMap, net::SocketAddr, sync::Arc, time::Duration};

use bincode::Options;
use hmac::{Hmac, Mac};
use k256::{ecdsa::SigningKey, sha2::Sha256};
use rand::Rng;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::{net::UdpSocket, runtime::Handle, task::JoinHandle};
use tokio_util::bytes::Bytes;

use crate::context::crypto::Verifier;

use super::{
    crypto::{DigestHash, Sign, Signer, Verify},
    ordered_multicast::{OrderedMulticast, Variant},
    OrderedMulticastReceivers, Receivers, ReplicaIndex, To,
};

#[derive(Debug, Clone)]
pub struct Config {
    pub num_faulty: usize,
    pub client_addrs: Vec<Addr>,
    pub replica_addrs: Vec<Addr>,
    pub signing_keys: HashMap<Addr, SigningKey>, // for replicas
    pub multicast_addr: Option<SocketAddr>,
    pub hmac: Hmac<Sha256>,
}

impl Config {
    pub fn new(
        client_addrs: impl Into<Vec<Addr>>,
        replica_addrs: impl Into<Vec<Addr>>,
        num_faulty: usize,
    ) -> Self {
        let client_addrs = client_addrs.into();
        let replica_addrs = replica_addrs.into();
        assert!(num_faulty * 3 < replica_addrs.len());
        let signing_keys = replica_addrs
            .iter()
            .enumerate()
            .map(|(index, &addr)| (addr, Self::k256(index as _)))
            .collect();
        Self {
            num_faulty,
            client_addrs,
            replica_addrs,
            signing_keys,
            multicast_addr: None,
            // simplified symmetrical keys setup
            // also reduce client-side overhead a little bit by only need to sign once for broadcast
            hmac: Hmac::new_from_slice("shared".as_bytes()).unwrap(),
        }
    }

    fn k256(index: ReplicaIndex) -> SigningKey {
        let k = format!("replica-{index}");
        let mut buf = [0; 32];
        buf[..k.as_bytes().len()].copy_from_slice(k.as_bytes());
        SigningKey::from_slice(&buf).unwrap()
    }
}

pub type Addr = SocketAddr;

#[derive(Debug, Clone)]
enum Event {
    Message(Addr, Addr, Vec<u8>),
    LoopbackMessage(Addr, Bytes),
    OrderedMulticastMessage(Addr, Vec<u8>),
    Timer(Addr, TimerId),
    Stop,
}

#[derive(Debug)]
pub struct Context {
    pub config: Arc<Config>,
    socket: Arc<UdpSocket>,
    runtime: Handle,
    pub source: Addr,
    signer: Signer,
    timer_id: TimerId,
    timer_tasks: HashMap<TimerId, JoinHandle<()>>,
    event: flume::Sender<Event>,
    rdv_event: flume::Sender<Event>,
}

impl Context {
    pub fn send<M, N>(&self, to: To, message: N)
    where
        M: Sign<N> + Serialize,
    {
        let message = M::sign(message, &self.signer);
        let buf = Bytes::from(bincode::options().serialize(&message).unwrap());
        if matches!(to, To::Loopback | To::AllReplicaWithLoopback) {
            self.event
                .send(Event::LoopbackMessage(self.source, buf.clone()))
                .unwrap()
        }
        use crate::context::Addr::Socket;
        match to {
            To::Addr(addr) => {
                let Socket(addr) = addr else { unimplemented!() };
                self.send_internal(addr, buf.clone())
            }
            To::Addrs(addrs) => {
                for addr in addrs {
                    let Socket(addr) = addr else { unimplemented!() };
                    self.send_internal(addr, buf.clone())
                }
            }
            To::Client(index) => self.send_internal(self.config.client_addrs[index as usize], buf),
            To::Clients(indexes) => {
                for index in indexes {
                    self.send_internal(self.config.client_addrs[index as usize], buf.clone())
                }
            }
            To::Replica(index) => {
                self.send_internal(self.config.replica_addrs[index as usize], buf)
            }
            To::AllReplica | To::AllReplicaWithLoopback => {
                for &addr in &self.config.replica_addrs {
                    if addr != self.source {
                        self.send_internal(addr, buf.clone())
                    }
                }
            }
            To::Loopback => {}
        }
    }

    fn send_internal(&self, addr: SocketAddr, buf: impl AsRef<[u8]> + Send + Sync + 'static) {
        let socket = self.socket.clone();
        self.runtime.spawn(async move {
            socket
                .send_to(buf.as_ref(), addr)
                .await
                .unwrap_or_else(|err| panic!("{err} target: {addr:?}"))
        });
    }

    pub fn send_ordered_multicast(&self, message: impl Serialize + DigestHash) {
        self.send_internal(
            self.config.multicast_addr.unwrap(),
            super::ordered_multicast::serialize(&message),
        )
    }

    pub fn idle_hint(&self) -> bool {
        self.event.is_empty()
    }
}

pub type TimerId = u32;

impl Context {
    pub fn set(&mut self, duration: Duration) -> TimerId {
        self.timer_id += 1;
        let id = self.timer_id;
        let event = self.rdv_event.clone();
        let source = self.source;
        let task = self.runtime.spawn(async move {
            loop {
                tokio::time::sleep(duration).await;
                event.send_async(Event::Timer(source, id)).await.unwrap()
            }
        });
        self.timer_tasks.insert(id, task);
        id
    }

    // only works in current thread runtime
    // in multi-thread runtime, there will always be a chance that the timer task wakes concurrent
    // to this `unset` call, then this call has no way to prevent false alarm
    // TODO mutual exclusive between `Receivers` callbacks and timer tasks
    pub fn unset(&mut self, id: TimerId) {
        let task = self.timer_tasks.remove(&id).unwrap();
        task.abort();
        let result = self.runtime.block_on(task);
        assert!(result.is_err())
    }
}

#[derive(Debug)]
pub struct Dispatch {
    runtime: Handle,
    verifier: Verifier,
    variant: Arc<Variant>,
    event: (flume::Sender<Event>, flume::Receiver<Event>),
    rdv_event: (flume::Sender<Event>, flume::Receiver<Event>),
    pub drop_rate: f64,
}

impl Dispatch {
    pub fn new(runtime: Handle, verifier: Verifier, variant: impl Into<Arc<Variant>>) -> Self {
        Self {
            runtime,
            verifier,
            variant: variant.into(),
            event: flume::unbounded(),
            rdv_event: flume::bounded(0),
            drop_rate: 0.,
        }
    }

    pub fn register<M>(
        &self,
        addr: Addr,
        config: impl Into<Arc<Config>>,
        signer: Signer,
    ) -> super::Context<M> {
        let socket = Arc::new(
            self.runtime
                .block_on(UdpSocket::bind(addr))
                .unwrap_or_else(|_| panic!("binding {addr:?}")),
        );
        socket.set_broadcast(true).unwrap();
        let context = Context {
            config: config.into(),
            socket: socket.clone(),
            runtime: self.runtime.clone(),
            source: addr,
            signer,
            timer_id: Default::default(),
            event: self.event.0.clone(),
            rdv_event: self.rdv_event.0.clone(),
            timer_tasks: Default::default(),
        };
        let event = self.event.0.clone();
        self.runtime.spawn(async move {
            let mut buf = vec![0; 65536];
            loop {
                let (len, remote) = socket.recv_from(&mut buf).await.unwrap();
                // `try_send` here to minimize rx process latency, avoid hardware packet dropping
                event
                    .try_send(Event::Message(addr, remote, buf[..len].to_vec()))
                    .unwrap()
            }
        });
        super::Context::Tokio(context)
    }
}

impl Dispatch {
    fn run_internal<R, M, N>(&self, receivers: &mut R, into: impl Fn(OrderedMulticast<N>) -> M)
    where
        R: Receivers<Message = M>,
        M: DeserializeOwned + Verify,
        N: DeserializeOwned + DigestHash,
    {
        let deserialize = |buf: &_| {
            bincode::options()
                .allow_trailing_bytes()
                .deserialize::<M>(buf)
                .unwrap()
        };
        let mut delegate = self.variant.delegate();
        let mut pace_count = 1;
        loop {
            if pace_count == 0 {
                // println!("* pace");
                delegate.on_pace(receivers, &self.verifier, &into);
                receivers.on_pace();
                pace_count = if self.event.0.is_empty() {
                    1
                } else {
                    self.event.0.len()
                };
                // println!("* pace count {pace_count}");
            }

            assert!(self.event.1.len() < 4096, "receivers overwhelmed");
            let event = flume::Selector::new()
                .recv(&self.event.1, Result::unwrap)
                .recv(&self.rdv_event.1, Result::unwrap)
                .wait();
            println!("{event:?}");
            use crate::context::Addr::Socket;
            match event {
                Event::Stop => break,
                Event::Message(receiver, remote, message) => {
                    pace_count -= 1;
                    if self.drop_rate != 0. && rand::thread_rng().gen_bool(self.drop_rate) {
                        continue;
                    }
                    let message = deserialize(&message);
                    message.verify(&self.verifier).unwrap();
                    receivers.handle(Socket(receiver), Socket(remote), message)
                }
                Event::LoopbackMessage(receiver, message) => {
                    pace_count -= 1;
                    receivers.handle_loopback(Socket(receiver), deserialize(&message))
                }
                Event::OrderedMulticastMessage(remote, message) => {
                    pace_count -= 1;
                    if self.drop_rate != 0. && rand::thread_rng().gen_bool(self.drop_rate) {
                        continue;
                    }
                    delegate.on_receive(
                        Socket(remote),
                        self.variant.deserialize(message),
                        receivers,
                        &self.verifier,
                        &into,
                    )
                }
                Event::Timer(receiver, id) => {
                    receivers.on_timer(Socket(receiver), super::TimerId::Tokio(id))
                }
            }
        }
    }

    pub fn run<M>(&self, receivers: &mut impl Receivers<Message = M>)
    where
        M: DeserializeOwned + Verify,
    {
        #[derive(Deserialize)]
        enum O {}
        impl DigestHash for O {
            fn hash(&self, _: &mut impl std::hash::Hasher) {
                unreachable!()
            }
        }
        self.run_internal::<_, _, O>(receivers, |_| unimplemented!())
    }
}

#[derive(Debug)]
pub struct OrderedMulticastDispatch(Dispatch);

impl std::ops::Deref for OrderedMulticastDispatch {
    type Target = Dispatch;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Dispatch {
    pub fn enable_ordered_multicast(self, addr: Addr) -> OrderedMulticastDispatch {
        let socket = self
            .runtime
            // .block_on(UdpSocket::bind(self.config.multicast_addr.unwrap()))
            .block_on(UdpSocket::bind(("0.0.0.0", addr.port())))
            .unwrap();
        let event = self.event.0.clone();
        self.runtime.spawn(async move {
            let mut buf = vec![0; 65536];
            loop {
                let (len, remote) = socket.recv_from(&mut buf).await.unwrap();
                event
                    .try_send(Event::OrderedMulticastMessage(remote, buf[..len].to_vec()))
                    .unwrap()
            }
        });
        OrderedMulticastDispatch(self)
    }
}

impl OrderedMulticastDispatch {
    pub fn run<M, N>(
        &self,
        receivers: &mut (impl Receivers<Message = M> + OrderedMulticastReceivers<Message = N>),
    ) where
        M: DeserializeOwned + Verify,
        N: DeserializeOwned + DigestHash,
        OrderedMulticast<N>: Into<M>,
    {
        self.run_internal(receivers, Into::into)
    }
}

pub struct DispatchHandle {
    stop: Box<dyn Fn() + Send + Sync>,
    stop_async:
        Box<dyn Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = ()>>> + Send + Sync>,
}

impl Dispatch {
    pub fn handle(&self) -> DispatchHandle {
        DispatchHandle {
            stop: Box::new({
                let rdv_event = self.rdv_event.0.clone();
                move || rdv_event.send(Event::Stop).unwrap()
            }),
            stop_async: Box::new({
                let rdv_event = self.rdv_event.0.clone();
                Box::new(move || {
                    let rdv_event = rdv_event.clone();
                    Box::pin(async move { rdv_event.send_async(Event::Stop).await.unwrap() }) as _
                })
            }),
        }
    }
}

impl DispatchHandle {
    pub fn stop(&self) {
        (self.stop)()
    }

    pub async fn stop_async(&self) {
        (self.stop_async)().await
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    fn false_alarm() {
        // let runtime = tokio::runtime::Builder::new_multi_thread()
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _enter = runtime.enter();
        let addr = SocketAddr::from(([127, 0, 0, 1], 10000));
        let config = Config::new([], [addr], 0);
        let dispatch = Dispatch::new(
            runtime.handle().clone(),
            Verifier::Nop,
            Variant::Unreachable,
        );

        #[derive(Serialize, Deserialize)]
        struct M;
        impl Verify for M {
            fn verify(&self, _: &Verifier) -> Result<(), crate::context::crypto::Invalid> {
                Ok(())
            }
        }

        let mut context = dispatch.register(
            addr,
            config.clone(),
            Signer::new_standard(None, config.hmac),
        );
        let id = context.set(Duration::from_millis(10));

        let handle = dispatch.handle();
        let event = dispatch.event.0.clone();
        std::thread::spawn(move || {
            runtime.block_on(async move {
                tokio::time::sleep(Duration::from_millis(9)).await;
                event
                    .send_async(Event::Message(
                        addr,
                        SocketAddr::from(([127, 0, 0, 1], 20000)),
                        bincode::options().serialize(&M).unwrap(),
                    ))
                    .await
                    .unwrap();
                tokio::time::sleep(Duration::from_millis(1)).await;
                handle.stop_async().await;
            });
            runtime.shutdown_background();
        });

        struct R(bool, crate::context::Context<M>, crate::context::TimerId);
        impl Receivers for R {
            type Message = M;

            fn handle(
                &mut self,
                _: crate::context::Addr,
                _: crate::context::Addr,
                M: Self::Message,
            ) {
                if !self.0 {
                    println!("unset");
                    self.1.unset(self.2);
                }
                self.0 = true;
            }

            fn handle_loopback(&mut self, _: crate::context::Addr, _: Self::Message) {
                unreachable!()
            }

            fn on_timer(&mut self, _: crate::context::Addr, _: crate::context::TimerId) {
                println!("alarm");
                assert!(!self.0);
            }
        }

        dispatch.run(&mut R(false, context, id));
    }

    #[test]
    fn false_alarm_100() {
        for _ in 0..100 {
            false_alarm();
            println!()
        }
    }
}
