use std::{
    net::{IpAddr, SocketAddr},
    time::{Duration, Instant},
};

use clap::Parser;
use neat_bincode::{de, ser};
use neat_core::{
    app::{Closure, FunctionalState},
    message::{Route, RouteLift, TransportLift},
    route::{self, ClientTable, ReplicaTable},
    App, Lift, {Drive, State, Wire},
};
use neat_pbft::{client, Client, Replica, Sign, Verify};
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

async fn run_clients(cli: Cli, route: ClientTable, replica_route: ReplicaTable) {
    use client::Message;

    let client_index = cli.client_index.unwrap();
    let mut clients = Vec::new();
    let mut ingress_tasks = Vec::new();
    for index in client_index..client_index + cli.client_count {
        let client_id = route.identity(index);
        let client_addr = route.lookup_addr(client_id);
        let client_wire = Wire::default();
        let socket = neat_tokio::udp::Socket::bind(client_addr).await;
        let ingress = Lift(de(), TransportLift)
            .install(Closure(|(_, message)| Message::Handle(message)).install(client_wire.state()));
        let workload = Workload {
            latencies: Default::default(),
            outstanding_start: Instant::now(),
            invoke: client_wire.state(),
        };
        let mut client = Client::new(
            client_id,
            cli.faulty_count,
            ser().install(
                Closure(Route::ToAll)
                    .install(route::External(replica_route.clone(), socket.clone())),
            ),
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
        ingress_tasks.push(spawn(async move { socket.start(ingress).await }));
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
    let request_wire = Wire::default();

    let replica_id = cli.replica_id.unwrap();
    let client_socket = udp::Socket::bind(replica_route.public_addr(replica_id)).await;
    let socket = udp::Socket::bind(replica_route.internal_addr(replica_id)).await;

    let app = Null
        .lift(neat_pbft::AppLift::new(replica_id))
        .install_filtered(
            Closure(move |(id, message)| (route.lookup_addr(id), message))
                .install(Lift(ser(), TransportLift).install(client_socket.clone())),
        );

    let mut replica = Replica::new(
        replica_id,
        replica_route.len(),
        cli.faulty_count,
        Sign::new(&replica_route, replica_id),
        app,
        ser()
            .lift_default::<RouteLift<_>>()
            .install(route::Internal(
                replica_route.clone(),
                replica_id,
                socket.clone(),
            )),
        neat_tokio::time::Control::default(),
    );

    let client_ingress = Lift(de::<neat_pbft::Request>(), TransportLift)
        .install(Closure(|(_, message)| message).install(request_wire.state()));
    let client_ingress = spawn(async move { client_socket.start(client_ingress).await });
    let ingress = Lift(de(), TransportLift).install(Closure(|(_, message)| message).install(
        Verify::new(&replica_route).install_filtered(
            Closure(|(message, signature)| (message, signature)).install(replica_wire.state()),
        ),
    ));
    let ingress = spawn(async move { socket.start(ingress).await });

    let mut request_drive = Drive::from(request_wire);
    let mut drive = Drive::from(replica_wire);
    let shutdown = ctrl_c();
    tokio::pin!(shutdown);
    loop {
        select! {
            message = request_drive.recv() => replica.update(message.expect("no active shutdown")),
            message = drive.recv() => replica.update(message.expect("no active shutdown")),
            timeout = replica.timeout.recv() => replica.update(timeout),
            result = &mut shutdown => break result.unwrap(),
        }
    }

    client_ingress.abort();
    ingress.abort();
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
