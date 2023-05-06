use std::{
    net::SocketAddr,
    time::{Duration, Instant},
};

use clap::Parser;
use larlis_barrier::{provide_barrier, use_barrier};
use larlis_bincode::{de, ser};
use larlis_core::{
    actor::{Drive, State, Wire},
    app::{Closure, PureState},
    route::{self, ClientTable},
    App,
};
use larlis_unreplicated::Replica;
use rand::{rngs::StdRng, SeedableRng};
use serde::{Deserialize, Serialize};
use tokio::{
    select,
    signal::ctrl_c,
    spawn,
    time::{sleep, timeout},
};

struct Null;

impl App for Null {
    fn update(&mut self, _: u32, _: &[u8]) -> Vec<u8> {
        Default::default()
    }
}

struct Workload<I> {
    latencies: Vec<Duration>,
    outstanding_start: Instant,
    invoke: I,
}

impl<I> State<'_> for Workload<I>
where
    I: for<'m> State<'m, Message = larlis_unreplicated::client::Message>,
{
    type Message = larlis_unreplicated::client::Result;

    fn update(&mut self, message: Self::Message) {
        assert_eq!(
            message,
            larlis_unreplicated::client::Result(Default::default())
        );
        let now = Instant::now();
        self.latencies.push(now - self.outstanding_start);
        self.outstanding_start = now;
        self.invoke
            .update(larlis_unreplicated::client::Message::Invoke(
                Default::default(),
            ));
    }
}

async fn run_clients(cli: Cli, route: route::ClientTable, replica_addr: SocketAddr) {
    use larlis_unreplicated::client::Message;

    let (Some(client_index), Some(client_count)) = (cli.client_index, cli.client_count) else {
        unreachable!()
    };
    let mut clients = Vec::new();
    let mut ingress_tasks = Vec::new();
    for index in client_index..client_index + client_count {
        let client_id = *route.identity(index);
        let client_addr = route.lookup_addr(&client_id);
        let client_wire = Wire::<Message>::default();
        let egress = larlis_udp::Out::bind(client_addr).await;
        let mut ingress = larlis_udp::In::new(
            &egress,
            de::<larlis_unreplicated::Reply>().install(
                Closure::from(|(_, message): (SocketAddr, _)| Message::Handle(message))
                    .install(client_wire.state()),
            ),
        );
        let workload = Workload {
            latencies: Default::default(),
            outstanding_start: Instant::now(),
            invoke: client_wire.state(),
        };
        let mut client = larlis_unreplicated::Client::new(
            client_id,
            Closure::from(move |message| (replica_addr, message)).install(ser().install(egress)),
            workload,
        );
        clients.push(spawn(async move {
            let now = Instant::now();
            client.result.outstanding_start = now;
            client_wire
                .state()
                .update(Message::Invoke(Default::default()));
            let mut client_drive = Drive::from(client_wire);

            //
            while Instant::now() - now
                < Duration::from_secs(cli.client_sec.unwrap()) + Duration::from_millis(100)
            {
                let result =
                    timeout(Duration::from_millis(100), client_drive.run(&mut client)).await;
                assert!(result.is_err());
                client.update(Message::Tick);
            }

            client
        }));
        ingress_tasks.push(spawn(async move { ingress.start().await }));
    }

    sleep(Duration::from_secs(cli.client_sec.unwrap())).await;

    for ingress in ingress_tasks {
        ingress.abort();
    }
    for (index, client) in (client_index..client_index + client_count).zip(clients) {
        let client = client.await.unwrap(); //
        let mut latencies = client.result.latencies;
        latencies.sort();
        println!(
            "{index},{},{}",
            latencies.len(),
            latencies
                .get(latencies.len() * 100 / 99)
                .unwrap_or(&Duration::ZERO)
                .as_secs_f64()
        );
    }
}

async fn run_replica(_cli: Cli, route: route::ClientTable, replica_addr: SocketAddr) {
    let egress = larlis_udp::Out::bind(replica_addr).await;

    let app = larlis_unreplicated::App(Null).install(
        Closure::from(move |(id, message)| (route.lookup_addr(&id), message))
            .install(ser().install(egress.clone())),
    );
    let mut replica = Replica::new(app);

    let mut ingress = larlis_udp::In::new(
        &egress,
        de().install(Closure::from(|(_, message)| message).install(&mut replica)),
    );
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
    #[clap(long)]
    client_sec: Option<u64>,
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
    let replica_addr = replica.unwrap();

    if cli.client_index.is_some() {
        run_clients(cli, route, replica_addr).await;
    } else {
        run_replica(cli, route, replica_addr).await;
    }
}
