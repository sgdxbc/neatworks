use std::{
    net::{IpAddr, SocketAddr},
    time::{Duration, Instant},
};

use clap::Parser;
use larlis_barrier::{provide_barrier, use_barrier};
use larlis_bincode::{de, ser};
use larlis_core::{
    actor::{Drive, State, Wire},
    app::{Closure, PureState},
    route::{self, ClientTable},
    App, Dispatch,
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

async fn run_clients_udp(cli: Cli, route: route::ClientTable, replica_addr: SocketAddr) {
    use larlis_unreplicated::client::Message;

    let client_index = cli.client_index.unwrap();
    let mut clients = Vec::new();
    let mut ingress_tasks = Vec::new();
    for index in client_index..client_index + cli.client_count {
        let client_id = *route.identity(index);
        let client_addr = route.lookup_addr(&client_id);
        let client_wire = Wire::default();
        let egress = larlis_udp::Out::bind(client_addr).await;
        let mut ingress = larlis_udp::In::new(
            &egress,
            de().install(
                Closure::from(|(_, message)| Message::Handle(message)).install(client_wire.state()),
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

async fn run_replica_udp(_cli: Cli, route: route::ClientTable, replica_addr: SocketAddr) {
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

async fn run_clients_tcp(cli: Cli, route: route::ClientTable, replica_addr: SocketAddr) {
    use larlis_unreplicated::client::Message;

    let client_index = cli.client_index.unwrap();
    let mut clients = Vec::new();
    let mut connections = Vec::new();
    for index in client_index..client_index + cli.client_count {
        let client_id = *route.identity(index);
        let client_addr = route.lookup_addr(&client_id);
        let client_wire = Wire::default();

        let mut connection = larlis_tcp::Connection::connect(
            client_addr,
            replica_addr,
            de().install(
                Closure::from(|(_, message)| Message::Handle(message)).install(client_wire.state()),
            ),
            Wire::default().state(),
        )
        .await;
        let mut dispatch = Dispatch::default();
        dispatch.insert_state(replica_addr, connection.out_state());
        connections.push(spawn(async move { connection.start().await }));

        let workload = Workload {
            latencies: Default::default(),
            outstanding_start: Instant::now(),
            invoke: client_wire.state(),
        };
        let mut client = larlis_unreplicated::Client::new(
            client_id,
            Closure::from(move |message| (replica_addr, message))
                .install(ser().install(Closure::from(From::from).install(dispatch))),
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
    }

    sleep(Duration::from_secs(cli.client_sec)).await;

    for connection in connections {
        connection.abort();
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

async fn run_replica_tcp(_cli: Cli, route: route::ClientTable, replica_addr: SocketAddr) {
    let replica_wire = Wire::default();
    let disconnected = Wire::default();

    let listener = larlis_tcp::Listener::bind(replica_addr);
    let mut dispatch = Dispatch::default();
    for _ in 0..route.len() {
        let mut connection = listener
            .accept(
                de().install(Closure::from(|(_, message)| message).install(replica_wire.state())),
                disconnected.state(),
            )
            .await;
        dispatch.insert_state(connection.remote_addr, connection.out_state());
        spawn(async move { connection.start().await });
    }

    let app = larlis_unreplicated::App(Null).install(
        Closure::from(move |(id, message)| (route.lookup_addr(&id), message))
            .install(ser().install(Closure::from(From::from).install(dispatch))),
    );
    let mut replica = Replica::new(app);

    let mut replica_drive = Drive::from(replica_wire);
    select! {
        _ = replica_drive.run(&mut replica) => {}  // gracefully shutdown
        result = ctrl_c() => result.unwrap(),
    }

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
    #[clap(long, default_value_t = 1)]
    client_count: usize,
    #[clap(long, default_value_t = 0)]
    barrier_count: usize,
    #[clap(long)]
    barrier_host: Option<IpAddr>,
    #[clap(long, default_value_t = 1)]
    client_sec: u64,
    #[clap(long)]
    tcp: bool,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    if cli.barrier_count != 0 {
        provide_barrier::<BarrierUser>(SocketAddr::from(([0, 0, 0, 0], 60000)), cli.barrier_count)
            .await;
        return;
    }

    let user = match cli.client_index {
        Some(index) => BarrierUser::Client(index, cli.client_count),
        None => BarrierUser::Replica,
    };
    let mut users = use_barrier(
        SocketAddr::from(([0, 0, 0, 0], 0)),
        SocketAddr::from((cli.barrier_host.unwrap(), 60000)),
        user,
    )
    .await;
    users.sort();

    let mut route = ClientTable::default();
    let mut replica = None;
    let mut rng = StdRng::seed_from_u64(0);
    for user in users {
        match user {
            (BarrierUser::Replica, addr) => {
                assert!(replica.is_none());
                replica = Some(SocketAddr::from((addr, 60002)));
            }
            (BarrierUser::Client(index, count), host) => {
                assert_eq!(index, route.len());
                route.add_host(host, count, &mut rng);
            }
        }
    }
    let replica_addr = replica.unwrap();

    if cli.client_index.is_some() {
        sleep(Duration::from_millis(100)).await;
    }
    match (cli.client_index, cli.tcp) {
        (Some(_), false) => run_clients_udp(cli, route, replica_addr).await,
        (None, false) => run_replica_udp(cli, route, replica_addr).await,
        (Some(_), true) => run_clients_tcp(cli, route, replica_addr).await,
        (None, true) => run_replica_tcp(cli, route, replica_addr).await,
    }
}
