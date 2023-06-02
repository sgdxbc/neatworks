use std::{
    net::{IpAddr, SocketAddr},
    time::{Duration, Instant},
};

use clap::Parser;
use neat_bincode::{de, ser};
use neat_core::{
    app::{Closure, FunctionalState},
    message::TransportLift,
    route::ClientTable,
    App, Dispatch, Lift, {Drive, State, Wire},
};
use neat_tokio::barrier::{provide_barrier, use_barrier};
use neat_unreplicated::{client, AppLift, Client, Replica};
use rand::{rngs::StdRng, SeedableRng};
use serde::{Deserialize, Serialize};
use tokio::{
    runtime, select,
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
        assert_eq!(
            message,
            neat_unreplicated::client::Result(Default::default())
        );
        let now = Instant::now();
        self.latencies.push(now - self.outstanding_start);
        self.outstanding_start = now;
        self.invoke
            .update(client::Message::Invoke(Default::default()));
    }
}

async fn run_clients_udp(
    cli: Cli,
    route: ClientTable,
    replica_addr: SocketAddr,
) -> Vec<(usize, Vec<Duration>)> {
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
            Closure(move |message| (replica_addr, message))
                .install(Lift(ser(), TransportLift).install(socket.clone())),
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
    let mut latencies = Vec::new();
    for (index, client) in (client_index..client_index + cli.client_count).zip(clients) {
        let client = client.await.unwrap(); //
        latencies.push((index, client.result.latencies));
    }
    latencies
}

async fn run_replica_udp(_cli: Cli, route: ClientTable, replica_addr: SocketAddr) {
    let socket = neat_tokio::udp::Socket::bind(replica_addr).await;

    let app = Null.lift(AppLift::default()).install_filtered(
        Closure(move |(id, message)| (route.lookup_addr(id), message))
            .install(Lift(ser(), TransportLift).install(socket.clone())),
    );
    let mut replica = Replica::new(app);

    let mut ingress =
        Lift(de(), TransportLift).install(Closure(|(_, message)| message).install(&mut replica));
    let shutdown = ctrl_c();
    select! {
        _ = socket.start(&mut ingress) => unreachable!(),
        result = shutdown => result.unwrap(),
    }

    // print some stats if needed
    let _replica = replica;
}

async fn run_clients_tcp(
    cli: Cli,
    route: ClientTable,
    replica_addr: SocketAddr,
) -> Vec<(usize, Vec<Duration>)> {
    use neat_unreplicated::client::Message;

    let client_index = cli.client_index.unwrap();
    let mut clients = Vec::new();
    let mut connections = Vec::new();
    for index in client_index..client_index + cli.client_count {
        let client_id = route.identity(index);
        let client_addr = route.lookup_addr(client_id);
        let client_wire = Wire::default();

        let mut connection = neat_tokio::tcp::Connection::connect(client_addr, replica_addr).await;
        let mut dispatch = Dispatch::default();
        dispatch.insert_state(replica_addr, connection.out_state());
        let client_state = client_wire.state();
        connections.push(spawn(async move {
            connection
                .start(
                    Lift(de(), TransportLift).install(
                        Closure(|(_, message)| Message::Handle(message)).install(client_state),
                    ),
                    Wire::default().state(),
                )
                .await
        }));

        let workload = Workload {
            latencies: Default::default(),
            outstanding_start: Instant::now(),
            invoke: client_wire.state(),
        };
        let mut client = Client::new(
            client_id,
            Closure(move |message| (replica_addr, message))
                .install(Lift(ser(), TransportLift).install(Closure(From::from).install(dispatch))),
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
    let mut latencies = Vec::new();
    for (index, client) in (client_index..client_index + cli.client_count).zip(clients) {
        let client = client.await.unwrap(); //
        latencies.push((index, client.result.latencies));
    }
    latencies
}

async fn run_replica_tcp(_cli: Cli, route: ClientTable, replica_addr: SocketAddr) {
    let replica_wire = Wire::default();

    let listener = neat_tokio::tcp::Listener::bind(replica_addr);
    let mut dispatch = Dispatch::default();
    for _ in 0..route.len() {
        let replica_state = replica_wire.state();
        let mut connection = listener.accept().await;
        dispatch.insert_state(connection.remote_addr, connection.out_state());
        spawn(async move {
            connection
                .start(
                    Lift(de(), TransportLift)
                        .install(Closure(|(_, message)| message).install(replica_state)),
                    Wire::default().state(),
                )
                .await
        });
    }

    let app = Null.lift(AppLift::default()).install_filtered(
        Closure(move |(id, message)| (route.lookup_addr(id), message))
            .install(Lift(ser(), TransportLift).install(Closure(From::from).install(dispatch))),
    );
    let mut replica = Replica::new(app);

    let mut replica_drive = Drive::from(replica_wire);
    replica_drive.run(&mut replica).await; // gracefully shutdown

    let _replica = replica;
}

async fn run_clients_tls(
    cli: Cli,
    route: ClientTable,
    replica_addr: SocketAddr,
) -> Vec<(usize, Vec<Duration>)> {
    use neat_unreplicated::client::Message;

    let client_index = cli.client_index.unwrap();
    let mut clients = Vec::new();
    let mut connections = Vec::new();
    for index in client_index..client_index + cli.client_count {
        let client_id = route.identity(index);
        let client_addr = route.lookup_addr(client_id);
        let client_wire = Wire::default();

        let connection = neat_tokio::tcp::Connection::connect(client_addr, replica_addr).await;
        let mut connection = neat_tokio::tls::Connector::default()
            .upgrade_client(connection)
            .await;
        let mut dispatch = Dispatch::default();
        dispatch.insert_state(replica_addr, connection.out_state());
        let client_state = client_wire.state();
        connections.push(spawn(async move {
            connection
                .start(
                    Lift(de(), TransportLift).install(
                        Closure(|(_, message)| Message::Handle(message)).install(client_state),
                    ),
                    Wire::default().state(),
                )
                .await
        }));

        let workload = Workload {
            latencies: Default::default(),
            outstanding_start: Instant::now(),
            invoke: client_wire.state(),
        };
        let mut client = Client::new(
            client_id,
            Closure(move |message| (replica_addr, message))
                .install(Lift(ser(), TransportLift).install(Closure(From::from).install(dispatch))),
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
    let mut latencies = Vec::new();
    for (index, client) in (client_index..client_index + cli.client_count).zip(clients) {
        let client = client.await.unwrap(); //
        latencies.push((index, client.result.latencies));
    }
    latencies
}

async fn run_replica_tls(_cli: Cli, route: ClientTable, replica_addr: SocketAddr) {
    let replica_wire = Wire::default();

    let listener = neat_tokio::tcp::Listener::bind(replica_addr);
    let acceptor = neat_tokio::tls::Acceptor::default();
    let mut dispatch = Dispatch::default();
    for _ in 0..route.len() {
        let connection = listener.accept().await;
        let mut connection = acceptor.upgrade_server(connection).await;
        dispatch.insert_state(connection.remote_addr, connection.out_state());
        let replica_state = replica_wire.state();
        spawn(async move {
            connection
                .start(
                    Lift(de(), TransportLift)
                        .install(Closure(|(_, message)| message).install(replica_state)),
                    Wire::default().state(),
                )
                .await
        });
    }

    let app = Null.lift(AppLift::default()).install_filtered(
        Closure(move |(id, message)| (route.lookup_addr(id), message))
            .install(Lift(ser(), TransportLift).install(Closure(From::from).install(dispatch))),
    );
    let mut replica = Replica::new(app);

    let mut replica_drive = Drive::from(replica_wire);
    replica_drive.run(&mut replica).await; // gracefully shutdown

    let _replica = replica;
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
enum BarrierUser {
    Replica,
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

    #[clap(long, default_value_t = 1)]
    thread_count: usize,

    #[clap(long)]
    tcp: bool,
    #[clap(long)]
    tls: bool,
}

async fn init() -> Option<(Cli, ClientTable, SocketAddr)> {
    let cli = Cli::parse();

    if cli.barrier_count != 0 {
        provide_barrier::<BarrierUser>(SocketAddr::from(([0, 0, 0, 0], 60000)), cli.barrier_count)
            .await;
        return None;
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
            (BarrierUser::Replica, host) => {
                assert!(replica.is_none());
                replica = Some(SocketAddr::from((host, 60002)));
            }
            (BarrierUser::Client(index, count), host) => {
                assert_eq!(index, route.len());
                route.add_host(host, count, &mut rng);
            }
        }
    }
    Some((cli, route, replica.unwrap()))
}

fn main() {
    let runtime = runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let Some((cli, route, replica_addr)) = runtime.block_on(init()) else {
        return;
    };
    if cli.client_index.is_some() {
        runtime.block_on(async move {
            sleep(Duration::from_millis(100)).await;
            let transport;
            let latencies = if cli.tls {
                transport = "tls";
                run_clients_tls(cli, route, replica_addr).await
            } else if cli.tcp {
                transport = "tcp";
                run_clients_tcp(cli, route, replica_addr).await
            } else {
                transport = "udp";
                run_clients_udp(cli, route, replica_addr).await
            };
            for (index, mut client_latencies) in latencies {
                client_latencies.sort_unstable();
                println!(
                    "{index},{transport},{},{}",
                    client_latencies.len(),
                    client_latencies
                        .get(client_latencies.len() / 2)
                        .unwrap_or(&Duration::ZERO)
                        .as_secs_f64()
                );
            }
        });
    } else {
        fn set_affinity() {
            use nix::{sched::sched_setaffinity, sched::CpuSet, unistd::Pid};
            use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
            static CPU_ID: AtomicUsize = AtomicUsize::new(0);
            let mut cpu_set = CpuSet::new();
            cpu_set.set(CPU_ID.fetch_add(1, SeqCst)).unwrap();
            sched_setaffinity(Pid::from_raw(0), &cpu_set).unwrap();
        }
        set_affinity();

        if cli.thread_count == 0 {
            runtime::Builder::new_current_thread().enable_all().build()
        } else {
            runtime::Builder::new_multi_thread()
                .worker_threads(cli.thread_count)
                .on_thread_start(set_affinity)
                .enable_all()
                .build()
        }
        .unwrap()
        .block_on(async move {
            if cli.tls {
                run_replica_tls(cli, route, replica_addr).await
            } else if cli.tcp {
                run_replica_tcp(cli, route, replica_addr).await
            } else {
                run_replica_udp(cli, route, replica_addr).await
            }
        })
    }
}
