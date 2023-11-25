use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    extract::State,
    response::IntoResponse,
    routing::{get, post},
    Json, Router, Server,
};
use helloween::{
    app::{null_session, NullWorkload},
    channel::{PromiseSender, PromiseSource},
    crypto::{Signer, Verifier},
    event_channel,
    net::UdpSocket,
    promise_channel,
    replication::{close_loop_session, AddrBook, SocketAddrBook},
    task::BackgroundMonitor,
    transport::Addr,
    unreplicated, Client, Replica,
};
use replication_control_messages as messages;
use tokio::{task::JoinSet, time::timeout};
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> helloween::Result<()> {
    std::env::set_var("RUST_BACKTRACE", "1");
    let port = std::env::args()
        .nth(1)
        .as_deref()
        .unwrap_or("10000")
        .parse::<u16>()?;
    let app = AppState::default();
    let shutdown = app.shutdown.clone();
    let app = Router::new()
        .route("/ok", get(|| async {}))
        .route("/run-client", post(run_client))
        .route("/take-client-result", post(take_client_result))
        .route("/run-replica", post(run_replica))
        .route("/reset-replica", post(reset_replica))
        .with_state(app.into());
    let signal_task = tokio::spawn({
        let shutdown = shutdown.clone();
        async move {
            let result = tokio::select! {
                result = tokio::signal::ctrl_c() => result,
                result = shutdown.cancelled() => Ok(result),
            };
            shutdown.cancel();
            result
        }
    });
    Server::bind(&([0, 0, 0, 0], port).into())
        .serve(app.into_make_service())
        .with_graceful_shutdown(shutdown.cancelled())
        .await?;
    signal_task.await??;
    Ok(())
}

#[derive(Default)]
struct AppState {
    client_result: Mutex<Option<(f32, Duration)>>,
    promise_reset: Mutex<Option<PromiseSender<PromiseSender<()>>>>,
    shutdown: CancellationToken,
}

type App = State<Arc<AppState>>;

async fn run_client(State(state): App, Json(payload): Json<messages::Client>) {
    tokio::spawn(run_client_internal(state, payload));
}

async fn run_client_internal(state: Arc<AppState>, config: messages::Client) {
    if let Err(err) = async {
        let mut monitor = BackgroundMonitor::default();
        let spawner = monitor.spawner();
        let stop_listen = CancellationToken::new();

        let addr_book = AddrBook::Socket(config.addr_book.into());
        let mut close_loop_sessions = JoinSet::new();
        let mut client_sessions = Vec::new();
        for id in config.id_range {
            let (event, source) = event_channel();
            let (invoke_event, invoke_source) = event_channel();

            let socket = UdpSocket::bind(addr_book.client_addr(id)?).await?;
            spawner.spawn({
                let socket = socket.clone();
                let stop = stop_listen.clone();
                async move {
                    socket
                        .listen_session::<unreplicated::Reply, _>(event, stop)
                        .await
                }
            });

            let client = Client {
                id,
                num_replica: config.num_replica,
                num_faulty: config.num_faulty,
                addr_book: addr_book.clone(),
                retry_interval: Duration::from_millis(100),
            };
            client_sessions.push(spawner.spawn(unreplicated::client_session(
                client.into(),
                invoke_source,
                source,
                socket.into_transport::<unreplicated::Request>(),
            )));

            close_loop_sessions.spawn(close_loop_session(
                NullWorkload,
                Duration::from_secs(10),
                invoke_event,
            ));
        }
        drop(spawner);

        let join_task = tokio::spawn(async move {
            let mut latencies = Vec::new();
            while let Some(result) = close_loop_sessions.join_next().await {
                latencies.extend(result??)
            }
            Ok::<_, helloween::Error>((
                latencies.len() as f32 / 10.,
                latencies.iter().sum::<Duration>() / latencies.len() as _,
            ))
        });
        let result = timeout(
            Duration::from_secs(10) + Duration::from_millis(100),
            monitor.wait_task(join_task),
        )
        .await????;
        for client_session in client_sessions {
            monitor.wait_task(client_session).await??
        }
        stop_listen.cancel();
        timeout(Duration::from_millis(100), monitor.wait()).await??;
        *state.client_result.lock().unwrap() = Some(result);
        Ok::<_, helloween::Error>(())
    }
    .await
    {
        eprintln!("{err}");
        eprint!("{}", err.backtrace());
        state.shutdown.cancel();
    }
}

async fn take_client_result(State(state): App) -> impl IntoResponse {
    Json(state.client_result.lock().unwrap().take())
}

async fn run_replica(State(state): App, Json(payload): Json<messages::Replica>) {
    let (promise_reset, reset) = promise_channel();
    *state.promise_reset.lock().unwrap() = Some(promise_reset);
    tokio::spawn(run_replica_internal(payload, reset, state.shutdown.clone()));
}

async fn run_replica_internal(
    config: messages::Replica,
    reset: PromiseSource<PromiseSender<()>>,
    shutdown: CancellationToken,
) {
    if let Err(err) = async {
        let mut monitor = BackgroundMonitor::default();
        let spawner = monitor.spawner();
        let mut addr_book = SocketAddrBook::from(config.addr_book);

        let socket = UdpSocket::bind(Addr::Socket(addr_book.remove_addr(config.id)?)).await?;
        let (listen_event, listen_source) = event_channel();
        let stop_listen = CancellationToken::new();
        spawner.spawn({
            let socket = socket.clone();
            let stop = stop_listen.clone();
            async move {
                socket
                    .listen_session::<unreplicated::Request, _>(listen_event, stop)
                    .await
            }
        });

        let (app_event, app_source) = event_channel();
        spawner.spawn(null_session(app_source));

        let mut verifiers = HashMap::new();
        for i in 0..config.num_replica {
            verifiers.insert(i as _, Verifier::new_hardcoded(i));
        }
        let replica = Replica {
            id: config.id,
            num_replica: config.num_replica,
            num_faulty: config.num_faulty,
            app: app_event,
            spawner: spawner.clone(),
            signer: Signer::new_hardcoded(config.id as _),
            verifiers,
            addr_book: AddrBook::Socket(addr_book),
        };
        let stop_replica = CancellationToken::new();
        let replica_task = spawner.spawn(unreplicated::replica_session(
            replica.into(),
            stop_replica.clone(),
            listen_source,
            socket.into_transport::<unreplicated::Reply>(),
        ));
        drop(spawner);

        let promise_ack = monitor.wait_task(reset).await??;
        stop_replica.cancel();
        replica_task.await?;
        stop_listen.cancel();
        timeout(Duration::from_millis(100), monitor.wait()).await??;
        promise_ack.resolve(());
        Ok::<_, helloween::Error>(())
    }
    .await
    {
        eprintln!("{err}");
        eprint!("{}", err.backtrace());
        shutdown.cancel()
    }
}

async fn reset_replica(State(state): App) {
    let (promise_ack, ack) = promise_channel();
    state
        .promise_reset
        .lock()
        .unwrap()
        .take()
        .unwrap()
        .resolve(promise_ack);
    ack.await.unwrap()
}
