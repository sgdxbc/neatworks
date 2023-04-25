use std::{net::SocketAddr, sync::Arc};

use bincode::Options;
use larlis_core::{actor::AdaptFn, route, App};
use larlis_unreplicated::Replica;
use tokio::{net::UdpSocket, select, signal::ctrl_c};

#[tokio::main]
async fn main() {
    println!("Hello, world!");
}

struct Null;

impl App for Null {
    fn execute(&mut self, _: u32, _: &[u8]) -> Vec<u8> {
        Default::default()
    }
}

async fn run_replica(route: route::ClientTable) {
    let socket = Arc::new(UdpSocket::bind("0.0.0.0:49999").await.unwrap());
    let outgress = (move |(client_id, reply)| {
        (
            route.lookup_addr(&client_id),
            bincode::options().serialize(&reply).unwrap(),
        )
    })
    .then(larlis_udp::Out(socket.clone()));
    let app = larlis_unreplicated::App(Null, Box::new(outgress));

    struct De(Replica);
    impl<'a> larlis_core::actor::State<'a> for De {
        type Message = (SocketAddr, &'a [u8]);

        fn update(&mut self, message: Self::Message) {
            let (_, buf) = message;
            let message = bincode::options()
                .allow_trailing_bytes()
                .deserialize(buf)
                .unwrap();
            self.0.update(message)
        }
    }

    let replica = De(Replica::new(Box::new(app)));
    let mut ingress = larlis_udp::In::new(socket, replica);

    select! {
        _ = ingress.start() => unreachable!(),
        result = ctrl_c() => result.unwrap(),
    }
}
