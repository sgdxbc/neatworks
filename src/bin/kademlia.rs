use std::{
    iter::repeat_with,
    sync::{Arc, Mutex},
};

use axum::{
    extract::State,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};

use helloween::{
    channel::SubscribeSource,
    crypto::{Signer, Verifier},
    event_channel,
    kademlia::{self, Buckets, Location, Peer, PeerRecord},
    net::UdpSocket,
    task::BackgroundMonitor,
    transport::Addr,
};
use kademlia_control_messages as messages;
use rand::{rngs::StdRng, seq::SliceRandom, Rng, SeedableRng};
use tokio::{net::TcpListener, task::JoinHandle};
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
        .route("/run-peer", post(run_peer))
        .route("/find-peer", post(find_peer))
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
    // we lose graceful shutdown (in a easy way) after upgrading axum to 0.7, so tentatively give up
    // on that
    // also not sure why only type checks with extra async wrapper
    let serve = async { axum::serve(TcpListener::bind(("0.0.0.0", port)).await?, app).await };
    tokio::select! {
        result = serve => result?,
        result = signal_task => result??,
    }
    Ok(())
}

#[derive(Default)]
struct AppState {
    shutdown: CancellationToken,
    session: Mutex<Option<JoinHandle<()>>>,
    subscribe_handles: Mutex<Vec<SubscribeHandle>>,
}

type SubscribeHandle = helloween::channel::SubscribeHandle<(Location, usize), Vec<PeerRecord>>;
type App = State<Arc<AppState>>;

async fn run_peer(State(state): App, Json(payload): Json<messages::Config>) {
    let mut session = state.session.lock().unwrap();
    assert!(session.is_none());
    let mut subcribe_handles = state.subscribe_handles.lock().unwrap();
    assert!(subcribe_handles.is_empty());
    let (handles, sources) = repeat_with(event_channel)
        .take(payload.num_host_peer)
        .unzip::<_, _, Vec<_>, Vec<_>>();
    *subcribe_handles = handles;
    *session = Some(tokio::spawn(run_peer_interal(
        payload,
        sources,
        state.shutdown.clone(),
    )))
}

async fn run_peer_interal(
    config: messages::Config,
    sources: Vec<SubscribeSource<(Location, usize), Vec<PeerRecord>>>,
    shutdown: CancellationToken,
) {
    if let Err(err) = async {
        let mut rng = StdRng::seed_from_u64(config.seed);
        let mut records = Vec::new();
        let mut run_signers = Vec::new();
        for (i, host) in config.hosts.iter().enumerate() {
            let host_signers = repeat_with(|| {
                let (secret_key, _) = secp256k1::generate_keypair(&mut rng);
                Signer::from(secret_key)
            })
            .take(config.num_host_peer)
            .collect::<Vec<_>>();
            records.extend(
                host_signers
                    .iter()
                    .enumerate()
                    .map(|(i, signer)| {
                        PeerRecord::new(signer, Addr::Socket((*host, 20000 + i as u16).into()))
                    })
                    .collect::<helloween::Result<Vec<_>>>()?,
            );
            if i == config.index {
                run_signers = host_signers.clone();
            }
        }

        let host = config.hosts[config.index];
        let mut rng = repeat_with(|| StdRng::from_seed(rng.gen()))
            .nth(config.index)
            .unwrap();
        let monitor = BackgroundMonitor::default();
        let spawner = monitor.spawner();
        for ((i, signer), subscribe_source) in run_signers.into_iter().enumerate().zip(sources) {
            let addr = Addr::Socket((host, 20000 + i as u16).into());
            let mut buckets = Buckets::new(PeerRecord::new(&signer, addr.clone())?);
            records.shuffle(&mut rng);
            for record in &records {
                buckets.insert(record.clone())
            }
            let peer = Peer {
                verifier: Verifier::from(&signer),
                signer: signer.into(),
                spawner: spawner.clone(),
            };
            let (message_event, message_source) = event_channel();
            let socket = UdpSocket::bind(addr).await?;
            spawner.spawn(socket.clone().listen_session(message_event));
            spawner.spawn(kademlia::session(
                peer.into(),
                buckets,
                subscribe_source,
                message_source,
                socket.into_transport::<kademlia::Message>(),
            ));
        }
        drop(spawner);
        monitor.wait().await?;
        Ok::<_, helloween::Error>(())
    }
    .await
    {
        println!("{err:}");
        println!("{}", err.backtrace());
        shutdown.cancel()
    }
}

async fn find_peer(
    State(state): App,
    Json(payload): Json<messages::FindPeer>,
) -> impl IntoResponse {
    let mut source = {
        let subscribe_handles = state.subscribe_handles.lock().unwrap();
        subscribe_handles[payload.index]
            .subscribe((payload.target, payload.count))
            .unwrap()
    };

    let mut collected = Vec::new();
    while let Some(result) = source.option_next().await {
        collected.push(
            result
                .into_iter()
                .map(|record| {
                    let Addr::Socket(addr) = record.addr else {
                        unimplemented!()
                    };
                    (record.id, addr)
                })
                .collect::<Vec<_>>(),
        );
    }
    Json(collected)
}
