use std::{
    env::args,
    net::SocketAddr,
    sync::{
        atomic::{AtomicU32, Ordering::SeqCst},
        Arc,
    },
    time::Duration,
};

use augustus::{
    event::{
        erased::{events::Init, session::Sender, Blanket, Session, Unify},
        SendEvent,
    },
    net::{
        session::{Dispatch, DispatchNet},
        SendMessage,
    },
};

use tokio::{
    task::JoinSet,
    time::{sleep, timeout_at, Instant},
};
use tracing::Level;
use tracing_subscriber::{
    filter::Targets, fmt::format::FmtSpan, layer::SubscriberExt, util::SubscriberInitExt as _,
};

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_span_events(FmtSpan::CLOSE)
        .with_max_level(Level::DEBUG)
        // .with_ansi(false)
        .finish()
        // .with("augustus=debug".parse::<Targets>()?)
        .with("warn".parse::<Targets>()?)
        .init();

    let num_peer = args().nth(1).map(|n| n.parse::<u16>()).unwrap_or(Ok(50))?;
    let multiplier = args().nth(2).map(|n| n.parse::<u8>()).unwrap_or(Ok(200))?;
    let expected = num_peer as u32 * num_peer as u32 * multiplier as u32;
    println!("expect {expected} messages");

    let count = Arc::new(AtomicU32::new(0));
    let mut sessions = JoinSet::new();
    for i in 0..num_peer {
        // let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", 3000 + i)).await?;
        let quic = augustus::net::session::Quic::new(SocketAddr::from(([0, 0, 0, 0], 3000 + i)))?;

        let mut control = Blanket(Unify(Dispatch::new(
            // augustus::net::session::Tcp::new(None)?,
            quic.clone(),
            {
                let count = count.clone();
                move |_: &_| {
                    count.fetch_add(1, SeqCst);
                    Ok(())
                }
            },
        )?));

        let mut control_session = Session::new();
        // sessions.spawn(augustus::net::session::tcp_accept_session(
        //     listener,
        sessions.spawn(augustus::net::session::quic_accept_session(
            quic,
            Sender::from(control_session.sender()),
        ));
        let mut net = DispatchNet(Sender::from(control_session.sender()));
        sessions.spawn(async move {
            Sender::from(control_session.sender()).send(Init)?;
            control_session.run(&mut control).await
        });
        sessions.spawn(async move {
            for j in 0..multiplier {
                for k in 0..num_peer {
                    net.send(
                        SocketAddr::from(([127, 0, 0, j + 1], 3000 + k)),
                        bytes::Bytes::from(b"hello".to_vec()),
                    )?;
                    sleep(Duration::from_millis(1)).await
                }
            }
            Ok(())
        });
    }

    let mut finished = 0;
    let mut deadline = Instant::now() + Duration::from_secs(1);
    while finished < expected {
        match timeout_at(deadline, sessions.join_next()).await {
            Ok(None) => break,
            Ok(Some(Ok(Ok(())))) => {}
            Ok(Some(result)) => {
                eprintln!("{result:?}");
                break;
            }
            Err(_) => {
                let count = count.swap(0, SeqCst);
                finished += count;
                eprintln!("{finished:6}/{expected:6} {count} messages/s");
                deadline += Duration::from_secs(1)
            }
        }
    }
    Ok(())
    // std::future::pending().await
}
