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
    crypto::{Signer, Verifier},
    model::{event_channel, Addr},
    net::UdpSocket,
    replication::{close_loop_session, AddrBook, SocketAddrBook, Stop},
    task::BackgroundMonitor,
    unreplicated, Client, Replica,
};
use replication_control_messages as control;
use tokio::{sync::oneshot, task::JoinSet};
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> helloween::Result<()> {
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
            let result = tokio::signal::ctrl_c().await;
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
    reset: Mutex<Option<oneshot::Sender<()>>>,
    shutdown: CancellationToken,
}

type App = State<Arc<AppState>>;

async fn run_client(State(state): App, Json(payload): Json<control::Client>) {
    tokio::spawn(run_client_internal(state, payload));
}

async fn run_client_internal(state: Arc<AppState>, client: control::Client) {
    let mut monitor = BackgroundMonitor::default();
    let spawner = monitor.spawner();
    let addr_book = AddrBook::Socket(client.addr_book.into());
    let mut sessions = JoinSet::new();
    for id in client.id_range {
        let (event, source) = event_channel();
        let (invoke_event, invoke_source) = event_channel();

        let socket = match async { UdpSocket::bind(addr_book.client_addr(id)?).await }.await {
            Ok(socket) => socket,
            Err(err) => {
                eprintln!("{err}");
                state.shutdown.cancel();
                return;
            }
        };
        spawner.spawn({
            let socket = socket.clone();
            async move { socket.listen_session::<unreplicated::Reply, _>(event).await }
        });

        let client = Client {
            id,
            num_replica: 1,
            num_faulty: 0,
            addr_book: addr_book.clone(),
            retry_interval: Duration::from_millis(100),
        };
        spawner.spawn(unreplicated::client_session(
            client.into(),
            invoke_source,
            source,
            socket.into_transport::<unreplicated::Request>(),
        ));

        sessions.spawn(close_loop_session(
            NullWorkload,
            Duration::from_secs(10),
            invoke_event,
        ));
    }

    let shutdown = state.shutdown.clone();
    spawner.spawn(async move {
        let mut latencies = Vec::new();
        while let Some(result) = sessions.join_next().await {
            latencies.extend(result??)
        }
        *state.client_result.lock().unwrap() = Some((
            latencies.len() as f32 / 10.,
            latencies.iter().sum::<Duration>() / latencies.len() as _,
        ));
        Ok(())
    });
    if let Err(err) = monitor.wait().await {
        eprintln!("{err}");
        shutdown.cancel()
    }
}

async fn take_client_result(State(state): App) -> impl IntoResponse {
    Json(state.client_result.lock().unwrap().take())
}

async fn run_replica(State(state): App, Json(payload): Json<control::Replica>) {
    let (reset_sender, reset_receiver) = oneshot::channel();
    *state.reset.lock().unwrap() = Some(reset_sender);
    tokio::spawn(run_replica_internal(
        payload,
        reset_receiver,
        state.shutdown.clone(),
    ));
}

async fn run_replica_internal(
    replica: control::Replica,
    reset: oneshot::Receiver<()>,
    shutdown: CancellationToken,
) {
    let mut monitor = BackgroundMonitor::default();
    let spawner = monitor.spawner();
    let (event, source) = event_channel();

    let mut addr_book = SocketAddrBook::from(replica.addr_book);
    let socket =
        match async { UdpSocket::bind(Addr::Socket(addr_book.remove_addr(replica.id)?)).await }
            .await
        {
            Ok(socket) => socket,
            Err(err) => {
                eprintln!("{err}");
                shutdown.cancel();
                return;
            }
        };
    spawner.spawn({
        let socket = socket.clone();
        let event = event.clone();
        async move {
            socket
                .listen_session::<unreplicated::Request, _>(event)
                .await
        }
    });

    let (app_event, app_source) = event_channel();
    spawner.spawn(null_session(app_source));

    let mut verifiers = HashMap::new();
    for i in 0..1 {
        verifiers.insert(i, Verifier::new_hardcoded(i as _));
    }
    let replica = Replica {
        id: replica.id,
        num_replica: 1,
        num_faulty: 0,
        app: app_event,
        spawner: spawner.clone(),
        signer: Signer::new_hardcoded(replica.id as _),
        verifiers,
        addr_book: AddrBook::Socket(addr_book),
    };
    spawner.spawn(unreplicated::replica_session(
        replica.into(),
        event.clone(),
        source,
        socket.into_transport::<unreplicated::Reply>(),
    ));

    if let Err(err) = async {
        tokio::select! {
            result = reset => result.unwrap(),
            err = monitor.wait() => Err(err.unwrap_err())?,
        }
        event.send(Stop.into())?;
        monitor.wait().await
    }
    .await
    {
        eprintln!("{err}");
        shutdown.cancel()
    }
}

async fn reset_replica(State(state): App) {
    state
        .reset
        .lock()
        .unwrap()
        .take()
        .unwrap()
        .send(())
        .unwrap()
}
