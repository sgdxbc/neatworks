use std::{
    net::{IpAddr, SocketAddr},
    time::{Duration, Instant},
};

use clap::Parser;
use tokio::{
    select,
    signal::ctrl_c,
    spawn,
    time::{sleep, timeout},
};
use wm_bincode::{de, ser};
use wm_core::{
    actor::{Drive, State, Wire},
    app::{Closure, PureState},
    route::{ClientTable, ReplicaTable},
    App, Dispatch,
};
use wm_pbft::{client, replica, Replica, Sign, Signature, ToReplica, Verify};

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
    I: for<'m> State<'m, Message = client::Message>,
{
    type Message = client::Result;

    fn update(&mut self, message: Self::Message) {
        assert_eq!(message, wm_pbft::client::Result(Default::default()));
        let now = Instant::now();
        self.latencies.push(now - self.outstanding_start);
        self.outstanding_start = now;
        self.invoke
            .update(client::Message::Invoke(Default::default()));
    }
}

async fn run_clients_udp(cli: Cli, route: ClientTable, replica_addr: SocketAddr) {
    use client::Message;

    let client_index = cli.client_index.unwrap();
    let mut clients = Vec::new();
    let mut ingress_tasks = Vec::new();
    for index in client_index..client_index + cli.client_count {
        let client_id = route.identity(index);
        let client_addr = route.lookup_addr(client_id);
        let client_wire = Wire::default();
        let egress = wm_udp::Out::bind(client_addr).await;
        let mut ingress = wm_udp::In::new(
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
        let mut client = wm_pbft::Client::new(
            client_id,
            cli.faulty_count,
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

async fn run_replica_udp(
    cli: Cli,
    route: ClientTable,
    replica_addr: SocketAddr,
    replica_route: ReplicaTable,
) {
    let replica_wire = Wire::default();

    let egress = wm_udp::Out::bind(replica_addr).await;

    let app = wm_pbft::App::new(cli.replica_id, Null).install_filtered(
        Closure::from(move |(id, message)| (route.lookup_addr(id), message))
            .install(ser().install(egress.clone())),
    );

    struct ReplicaEgress<S> {
        route: ReplicaTable,
        id: u8,
        state: S,
    }
    impl<'m, S> State<'m> for ReplicaEgress<S>
    where
        S: State<'m, Message = (SocketAddr, (ToReplica, Signature))>,
    {
        type Message = replica::Egress<(ToReplica, Signature)>;

        fn update(&mut self, message: Self::Message) {
            match message {
                replica::Egress::To(replica_id, message) => {
                    assert_ne!(replica_id, self.id);
                    self.state
                        .update((self.route.lookup_addr(replica_id), message))
                }
                replica::Egress::ToAll(message) => {
                    for id in 0..self.route.len() as u8 {
                        if id == self.id {
                            continue;
                        }
                        self.state
                            .update((self.route.lookup_addr(id), message.clone()))
                    }
                }
            }
        }
    }

    let (mut waker, control) =
        wm_time::new(Closure::from(ToReplica::Timeout).install(replica_wire.state()));

    let mut replica = Replica::new(
        cli.replica_id,
        replica_route.len(),
        cli.faulty_count,
        app,
        // TODO lift `ser()` as well
        replica::EgressLift(Sign::new(&replica_route, cli.replica_id)).install(ReplicaEgress {
            route: replica_route.clone(),
            id: cli.replica_id,
            state: ser().install(egress.clone()),
        }),
        control.install(Dispatch::default()),
    );

    let mut ingress = wm_udp::In::new(
        &egress,
        de().install(
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
    replica_id: u8,
    #[clap(long, default_value_t = 1)]
    faulty_count: usize,
}

fn main() {
    println!("Hello, world!");
}
