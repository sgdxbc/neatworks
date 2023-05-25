use std::{
    net::{IpAddr, SocketAddr},
    time::{Duration, Instant},
};

use clap::Parser;
use neat_bincode::{de, ser};
use neat_core::{
    app::{Closure, FunctionalState},
    message::{Egress, EgressLift, Transport, TransportLift},
    route::{ClientTable, ReplicaTable},
    App, Dispatch, Lift, {Drive, State, Wire},
};
use neat_pbft::{client, Client, Replica, Sign, ToReplica, Verify};
use neat_tokio::{
    barrier::{provide_barrier, use_barrier},
    udp,
};
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

impl<I> State<client::Result> for Workload<I>
where
    I: State<client::Message>,
{
    fn update(&mut self, message: client::Result) {
        assert_eq!(message, neat_pbft::client::Result(Default::default()));
        let now = Instant::now();
        self.latencies.push(now - self.outstanding_start);
        self.outstanding_start = now;
        self.invoke
            .update(client::Message::Invoke(Default::default()));
    }
}

struct EgressRoute<R, S> {
    strategy: R,
    route: ReplicaTable,
    state: S,
}

trait RouteStrategy {
    fn lookup_addr(route: &ReplicaTable, id: u8) -> SocketAddr;
    fn is_exclude(&self, id: u8) -> bool;
}

struct ClientEgress;

impl RouteStrategy for ClientEgress {
    fn lookup_addr(route: &ReplicaTable, id: u8) -> SocketAddr {
        route.public_addr(id)
    }

    fn is_exclude(&self, _: u8) -> bool {
        false
    }
}

struct ReplicaEgress(u8);

impl RouteStrategy for ReplicaEgress {
    fn lookup_addr(route: &ReplicaTable, id: u8) -> SocketAddr {
        route.internal_addr(id)
    }

    fn is_exclude(&self, id: u8) -> bool {
        id == self.0
    }
}

impl<R, S> State<Egress<u8, Vec<u8>>> for EgressRoute<R, S>
where
    R: RouteStrategy,
    S: State<Transport<Vec<u8>>>,
{
    fn update(&mut self, message: Egress<u8, Vec<u8>>) {
        match message {
            Egress::To(replica_id, message) => {
                assert!(!self.strategy.is_exclude(replica_id));
                let addr = R::lookup_addr(&self.route, replica_id);
                self.state.update((addr, message))
            }
            Egress::ToAll(message) => {
                for id in 0..self.route.len() as u8 {
                    if self.strategy.is_exclude(id) {
                        continue;
                    }
                    self.state
                        .update((R::lookup_addr(&self.route, id), message.clone()))
                }
            }
        }
    }
}

async fn run_clients(cli: Cli, route: ClientTable, replica_route: ReplicaTable) {
    use client::Message;

    let client_index = cli.client_index.unwrap();
    let mut clients = Vec::new();
    let mut ingress_tasks = Vec::new();
    for index in client_index..client_index + cli.client_count {
        let client_id = route.identity(index);
        let client_addr = route.lookup_addr(client_id);
        let client_wire = Wire::default();
        let egress = udp::Out::bind(client_addr).await;
        let mut ingress = udp::In::new(
            &egress,
            // Closure(|(_, message)| message).install(
            //     de()
            Lift(de(), TransportLift).install(
                Closure(|(_, message)| message)
                    .install(Closure(Message::Handle).install(client_wire.state())),
            ),
        );
        let workload = Workload {
            latencies: Default::default(),
            outstanding_start: Instant::now(),
            invoke: client_wire.state(),
        };
        let mut client = Client::new(
            client_id,
            cli.faulty_count,
            ser().install(Closure(Egress::ToAll).install(EgressRoute {
                strategy: ClientEgress,
                route: replica_route.clone(),
                state: egress,
            })),
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
                < Duration::from_secs(cli.client_sec) + Duration::from_millis(100)
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

    sleep(Duration::from_secs(cli.client_sec)).await;

    for ingress in ingress_tasks {
        ingress.abort();
    }
    for (index, client) in (client_index..client_index + cli.client_count).zip(clients) {
        let client = client.await.unwrap(); //
        let mut latencies = client.result.latencies;
        latencies.sort();
        println!(
            "{index},{},{}",
            latencies.len(),
            latencies
                .get(latencies.len() * 99 / 100)
                .unwrap_or(&Duration::ZERO)
                .as_secs_f64()
        );
    }
}

async fn run_replica(cli: Cli, route: ClientTable, replica_route: ReplicaTable) {
    let replica_wire = Wire::default();
    let timeout_wire = Wire::default();

    let replica_id = cli.replica_id.unwrap();
    let client_egress = udp::Out::bind(replica_route.public_addr(replica_id)).await;
    let egress = udp::Out::bind(replica_route.internal_addr(replica_id)).await;

    let app = Null
        .lift(neat_pbft::AppLift::new(replica_id))
        .install_filtered(
            Closure(move |(id, message)| (route.lookup_addr(id), message))
                .install(Lift(ser(), TransportLift).install(client_egress.clone())),
        );

    let (mut waker, control) = neat_tokio::time::new(timeout_wire.state());
    let mut replica = Replica::new(
        replica_id,
        replica_route.len(),
        cli.faulty_count,
        app,
        Sign::new(&replica_route, replica_id)
            .lift_default::<EgressLift<_>>()
            .install(ser().lift_default::<EgressLift<_>>().install(EgressRoute {
                strategy: ReplicaEgress(replica_id),
                route: replica_route.clone(),
                state: egress.clone(),
            })),
        control.install(timeout_wire.state()),
    );

    let mut client_ingress = udp::In::new(
        &client_egress,
        Lift(de(), TransportLift).install(
            Closure(|(_, message)| ToReplica::Request(message)).install(replica_wire.state()),
        ),
    );
    let client_ingress = spawn(async move { client_ingress.start().await });
    let mut ingress = udp::In::new(
        &egress,
        Lift(de(), TransportLift).install(
            Closure(|(_, message)| message)
                .install(Verify::new(&replica_route, replica_wire.state())),
        ),
    );
    let ingress = spawn(async move { ingress.start().await });
    let mut timeout = Drive::from(timeout_wire);
    let timeout = spawn(async move { timeout.run(Dispatch::default()).await });

    let mut drive = Drive::from(replica_wire);
    loop {
        select! {
            message = drive.recv() => replica.update(message.unwrap()),
            timeout = waker.recv() => replica.update(timeout.unwrap()),
            result = ctrl_c() => break result.unwrap(),
        }
    }

    client_ingress.abort();
    ingress.abort();
    timeout.abort();
    // print some stats if needed
    let _replica = replica;
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
enum BarrierUser {
    Replica(u8),
    Client(usize, usize),
}

#[derive(Parser)]
struct Cli {
    #[clap(long)]
    local: bool,

    #[clap(long, default_value_t = 0)]
    barrier_count: usize,
    #[clap(long)]
    barrier_host: Option<IpAddr>,

    #[clap(long)]
    client_index: Option<usize>,
    #[clap(long, default_value_t = 1)]
    client_count: usize,
    #[clap(long, default_value_t = 1)]
    client_sec: u64,

    #[clap(long)]
    replica_id: Option<u8>,
    #[clap(long, default_value_t = 1)]
    faulty_count: usize,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    if cli.barrier_count != 0 {
        provide_barrier::<BarrierUser>(SocketAddr::from(([0, 0, 0, 0], 60000)), cli.barrier_count)
            .await;
        return;
    }

    let user = match (cli.client_index, cli.replica_id) {
        (Some(index), None) => BarrierUser::Client(index, cli.client_count),
        (None, Some(id)) => BarrierUser::Replica(id),
        _ => unimplemented!(),
    };
    let mut users = use_barrier(
        match user {
            BarrierUser::Replica(id) if cli.local => SocketAddr::from(([127, 0, 0, 100 + id], 0)),
            _ => SocketAddr::from(([0, 0, 0, 0], 0)),
        },
        SocketAddr::from((cli.barrier_host.unwrap(), 60000)),
        user,
    )
    .await;
    users.sort();

    let mut route = ClientTable::default();
    let mut replica_route = ReplicaTable::default();
    let mut rng = StdRng::seed_from_u64(0);
    for user in users {
        match user {
            (BarrierUser::Replica(id), host) => {
                assert_eq!(id as usize, replica_route.len());
                replica_route.add(host, &mut rng);
            }
            (BarrierUser::Client(index, count), host) => {
                assert_eq!(index, route.len());
                route.add_host(host, count, &mut rng);
            }
        }
    }

    if cli.client_index.is_some() {
        sleep(Duration::from_millis(100)).await;
        run_clients(cli, route, replica_route).await;
    } else {
        run_replica(cli, route, replica_route).await;
    }
}
