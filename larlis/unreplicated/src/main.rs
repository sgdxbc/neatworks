use std::{marker::PhantomData, net::SocketAddr, ops::Range, sync::Arc};

use bincode::Options;
use larlis_core::{
    app::{Closure, PureState},
    route, App,
};
use larlis_unreplicated::{Replica, Reply};
use serde::de::DeserializeOwned;
use tokio::{net::UdpSocket, select, signal::ctrl_c};

#[tokio::main]
async fn main() {
    println!("Hello, world!");
}

struct Null;

impl App for Null {
    fn update(&mut self, _: u32, _: &[u8]) -> Vec<u8> {
        Default::default()
    }
}

struct De<M>(PhantomData<M>);

impl<'i, M> PureState<'i> for De<M>
where
    M: DeserializeOwned,
{
    type Input = (SocketAddr, &'i [u8]);
    type Output<'output> = (SocketAddr, M) where Self: 'output;

    fn update(&mut self, input: Self::Input) -> Self::Output<'_> {
        let (addr, buf) = input;
        let message = bincode::options()
            .allow_trailing_bytes()
            .deserialize(buf)
            .unwrap();
        (addr, message)
    }
}

async fn run_clients(route: route::ClientTable, indexes: Range<usize>) {}

async fn run_replica(route: route::ClientTable) {
    let socket = Arc::new(UdpSocket::bind("0.0.0.0:49999").await.unwrap());

    struct AppOut(larlis_udp::Out, route::ClientTable);
    impl larlis_core::actor::State<'_> for AppOut {
        type Message = (u32, Reply);

        fn update(&mut self, message: Self::Message) {
            let (client_id, reply) = message;
            let message = (
                self.1.lookup_addr(&client_id),
                bincode::options().serialize(&reply).unwrap(),
            );
            self.0.update(message)
        }
    }

    let app = larlis_unreplicated::App(Null)
        .install(AppOut(larlis_udp::Out(socket.clone()), Default::default()));

    let replica = De(PhantomData).install(Closure::from(|(_, m)| m).install(Replica::new(app)));
    let mut ingress = larlis_udp::In {
        socket,
        state: replica,
    };

    select! {
        _ = ingress.start() => unreachable!(),
        result = ctrl_c() => result.unwrap(),
    }

    // print some stats if needed
    let _replica = ingress.state.1 .1;
}
