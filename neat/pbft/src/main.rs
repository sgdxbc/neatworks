use std::{
    net::{IpAddr, SocketAddr},
    time::{Duration, Instant},
};

use clap::Parser;
use neat_barrier::{provide_barrier, use_barrier};
use neat_bincode::{de, ser};
use neat_core::{
    actor::{Drive, State, Wire},
    app::{Closure, FunctionalState},
    message::Transport,
    route::{ClientTable, ReplicaTable},
    transport, App, Dispatch,
};
use neat_pbft::{client, replica, Replica, Sign, ToReplica, Verify};
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

struct EgressRoute<S> {
    route: ReplicaTable,
    addr: SocketAddr,
    state: S,
}

impl<S> State<replica::Egress<Vec<u8>>> for EgressRoute<S>
where
    S: State<Transport<Vec<u8>>>,
{
    fn update(&mut self, message: replica::Egress<Vec<u8>>) {
        match message {
            replica::Egress::To(replica_id, message) => {
                let addr = self.route.lookup_addr(replica_id);
                assert_ne!(addr, self.addr);
                self.state.update((addr, message))
            }
            replica::Egress::ToAll(message) => {
                for id in 0..self.route.len() as u8 {
                    let addr = self.route.lookup_addr(id);
                    if addr == self.addr {
                        continue;
                    }
                    self.state.update((addr, message.clone()))
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
        let egress = neat_udp::Out::bind(client_addr).await;
        let mut ingress = neat_udp::In::new(
            &egress,
            transport::Lift(de()).install(
                Closure::from(|(_, message)| Message::Handle(message)).install(client_wire.state()),
            ),
        );
        let workload = Workload {
            latencies: Default::default(),
            outstanding_start: Instant::now(),
            invoke: client_wire.state(),
        };
        let mut client = neat_pbft::Client::new(
            client_id,
            cli.faulty_count,
            Closure::from(replica::Egress::ToAll).install(replica::EgressLift(ser()).install(
                EgressRoute {
                    route: replica_route.clone(),
                    addr: client_addr,
                    state: egress,
                },
            )),
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
    let replica_id = cli.replica_id.unwrap();
    let replica_addr = replica_route.lookup_addr(replica_id);
    let replica_wire = Wire::default();

    let egress = neat_udp::Out::bind(replica_addr).await;

    let app = neat_pbft::App::new(replica_id, Null).install_filtered(
        Closure::from(move |(id, message)| (route.lookup_addr(id), message))
            .install(transport::Lift(ser()).install(egress.clone())),
    );

    let (mut waker, control) =
        neat_time::new(Closure::from(ToReplica::Timeout).install(replica_wire.state()));

    let mut replica = Replica::new(
        replica_id,
        replica_route.len(),
        cli.faulty_count,
        app,
        replica::EgressLift(Sign::new(&replica_route, replica_id)).install(
            replica::EgressLift(ser()).install(EgressRoute {
                route: replica_route.clone(),
                addr: replica_addr,
                state: egress.clone(),
            }),
        ),
        control.install(Dispatch::default()),
    );

    let mut ingress = neat_udp::In::new(
        &egress,
        transport::Lift(de()).install(
            Closure::from(|(_, message)| message)
                .install(Verify::new(&replica_route, replica_wire.state())),
        ),
    );
    let ingress = spawn(async move { ingress.start().await });
    let waker = spawn(async move { waker.start().await });

    let mut drive = Drive::from(replica_wire);
    select! {
        _ = drive.run(&mut replica) => unreachable!(),
        result = ctrl_c() => result.unwrap(),
    }

    ingress.abort();
    waker.abort();
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
        SocketAddr::from(([0, 0, 0, 0], 0)),
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
                replica_route.add(SocketAddr::new(host, 0), &mut rng);
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
