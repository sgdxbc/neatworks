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
    app::null_session,
    crypto::{Signer, Verifier},
    model::event_channel,
    net::UdpSocket,
    replication::Stop,
    task::BackgroundMonitor,
    unreplicated, Replica,
};
use replication_control_messages as control;
use tokio::sync::oneshot;
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
        .route("/ok", get(ok))
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
    client_result: Mutex<Option<(usize, Duration)>>,
    reset: Mutex<Option<oneshot::Sender<()>>>,
    shutdown: CancellationToken,
}

type App = State<Arc<AppState>>;

async fn ok() {}

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

    let Ok(socket) = tokio::net::UdpSocket::bind(replica.addr).await else {
        eprintln!("bind address {} fail", replica.addr);
        shutdown.cancel();
        return;
    };
    let socket = UdpSocket::from(Arc::new(socket));
    spawner.spawn({
        let socket = socket.clone();
        let event = event.clone();
        async move { socket.listen_loop::<unreplicated::Request, _>(event).await }
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
        addr_book: replica.addr_book.into(),
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
