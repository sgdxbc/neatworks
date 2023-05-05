use std::{net::SocketAddr, ops::Range, sync::Arc};

use clap::Parser;
use larlis_barrier::{provide_barrier, use_barrier};
use larlis_bincode::{de, ser};
use larlis_core::{
    app::{Closure, PureState},
    route::{self, ClientTable},
    App,
};
use larlis_unreplicated::Replica;
use rand::{rngs::StdRng, SeedableRng};
use serde::{Deserialize, Serialize};
use tokio::{net::UdpSocket, select, signal::ctrl_c};

struct Null;

impl App for Null {
    fn update(&mut self, _: u32, _: &[u8]) -> Vec<u8> {
        Default::default()
    }
}

async fn run_clients(route: route::ClientTable, indexes: Range<usize>) {}

async fn run_replica(route: route::ClientTable) {
    let socket = Arc::new(UdpSocket::bind("0.0.0.0:60002").await.unwrap());

    let app = larlis_unreplicated::App(Null).install(
        Closure::from(move |(id, message)| (route.lookup_addr(&id), message))
            .install(ser().install(larlis_udp::Out(socket.clone()))),
    );
    let mut replica = Replica::new(app);

    let mut ingress = larlis_udp::In {
        socket,
        state: de().install(Closure::from(|(_, message)| message).install(&mut replica)),
    };
    select! {
        _ = ingress.start() => unreachable!(),
        result = ctrl_c() => result.unwrap(),
    }

    // print some stats if needed
    let _replica = replica;
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
enum BarrierUser {
    Replica,
    Client(usize, usize),
}

#[derive(Parser)]
struct Cli {
    #[clap(long)]
    client_index: Option<usize>,
    #[clap(long)]
    client_count: Option<usize>,
    #[clap(long, default_value_t = 0)]
    barrier_count: usize,
    #[clap(long)]
    barrier_addr: Option<SocketAddr>,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    if cli.barrier_count != 0 {
        provide_barrier::<BarrierUser>(SocketAddr::from(([0, 0, 0, 0], 60000)), cli.barrier_count)
            .await;
        return;
    }

    let user = match (cli.client_index, cli.client_count) {
        (Some(index), Some(count)) => BarrierUser::Client(index, count),
        (None, None) => BarrierUser::Replica,
        _ => unimplemented!(),
    };
    let mut users = use_barrier(
        SocketAddr::from(([0, 0, 0, 0], 60001)),
        cli.barrier_addr.unwrap(),
        user,
    )
    .await;
    users.sort();

    let mut route = ClientTable::default();
    let mut replica = None;
    let mut rng = StdRng::seed_from_u64(0);
    for user in users {
        match user {
            (BarrierUser::Replica, mut addr) => {
                assert!(replica.is_none());
                addr.set_port(60002);
                replica = Some(addr);
            }
            (BarrierUser::Client(index, count), addr) => {
                assert_eq!(index, route.len());
                route.add_host(addr.ip(), count, &mut rng);
            }
        }
    }

    match (cli.client_index, cli.client_count) {
        (Some(index), Some(count)) => run_clients(route, index..count).await,
        (None, None) => run_replica(route).await,
        _ => unimplemented!(),
    }
}
