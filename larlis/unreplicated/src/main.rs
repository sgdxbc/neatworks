use std::{net::SocketAddr, ops::Range, sync::Arc};

use larlis_bincode::{de, ser};
use larlis_core::{
    actor::State,
    app::{Closure, PureState},
    route, App,
};
use larlis_unreplicated::{Replica, Reply};
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

async fn run_clients(route: route::ClientTable, indexes: Range<usize>) {}

async fn run_replica(route: route::ClientTable) {
    let socket = Arc::new(UdpSocket::bind("0.0.0.0:49999").await.unwrap());

    struct AppOut<O>(O, route::ClientTable);
    impl<O: for<'m> State<'m, Message = (SocketAddr, Reply)>> larlis_core::actor::State<'_>
        for AppOut<O>
    {
        type Message = (u32, Reply);

        fn update(&mut self, message: Self::Message) {
            let (client_id, reply) = message;
            self.0.update((self.1.lookup_addr(&client_id), reply))
        }
    }

    let app = larlis_unreplicated::App(Null).install(AppOut(
        ser().install(larlis_udp::Out(socket.clone())),
        Default::default(), // TODO
    ));

    let replica = de().install(Closure::from(|(_, m)| m).install(Replica::new(app)));
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
