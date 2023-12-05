use std::{convert::identity, io::Write, net::SocketAddr, sync::OnceLock, time::Duration};

use kademlia_control_messages::{BootstrapConfig, Config, FindPeer, Peer};
use tokio::{
    task::JoinSet,
    time::{sleep, timeout},
};
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    CLIENT
        .set(
            reqwest::Client::builder()
                .timeout(Duration::from_secs(1))
                .build()?,
        )
        .unwrap();
    session().await
}

static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

async fn peer_session(url: String, shutdown: CancellationToken) -> anyhow::Result<()> {
    let client = CLIENT.get().unwrap();
    while timeout(Duration::from_secs(1), shutdown.cancelled())
        .await
        .is_err()
    {
        client
            .get(format!("{url}/ok"))
            .send()
            .await?
            .error_for_status()?;
    }
    Ok(())
}

async fn find_session(url: String, find_peer: FindPeer) -> anyhow::Result<()> {
    let client = CLIENT.get().unwrap();
    let id = client
        .post(format!("{url}/find-peer"))
        .json(&find_peer)
        .send()
        .await?
        .error_for_status()?
        .json::<usize>()
        .await?;
    println!("[{url} #{id}] submitted");
    while {
        let (update, finished) = client
            .get(format!("{url}/find-peer/{id}"))
            .send()
            .await?
            .error_for_status()?
            .json::<(Vec<Vec<([u8; 32], SocketAddr)>>, bool)>()
            .await?;
        let mut stdout = std::io::stdout().lock();
        for message in update {
            writeln!(stdout, "[{url} #{id}] update")?;
            for (id, addr) in message {
                writeln!(stdout, "  {} {addr}", hex_string(&id))?
            }
        }
        if finished {
            writeln!(stdout, "[{url} #{id}] finished")?
        }
        !finished
    } {}
    Ok(())
}

fn hex_string(id: &[u8; 32]) -> String {
    id.map(|n| format!("{n:02x}")).join("")
}

async fn session() -> anyhow::Result<()> {
    let client = CLIENT.get().unwrap();
    let urls = [
        "http://127.0.0.1:10000",
        "http://127.0.0.1:10001",
        "http://127.0.0.1:10002",
        "http://127.0.0.1:10003",
    ];
    let mut peer_sessions = JoinSet::new();
    let shutdown = CancellationToken::new();
    let mut peers = Vec::new();
    for (i, url) in urls.into_iter().enumerate().take(3) {
        let config = Config {
            seed: 117418,
            num_host_peer: 3,
            hosts: vec![[127, 0, 0, 1].into()],
            index: (0, i),
        };
        let peer = client
            .post(format!("{url}/run-peer"))
            .json(&config)
            .send()
            .await?
            .error_for_status()?
            .json::<Peer>()
            .await?;
        println!("peer {} {}", hex_string(&peer.id), peer.addr);
        peers.push(peer);
        peer_sessions.spawn(peer_session(url.into(), shutdown.clone()));
    }

    let mut tasks = JoinSet::new();
    for peer in &peers {
        let find_peer = FindPeer {
            target: peer.id,
            count: 1,
        };
        for url in urls.into_iter().take(3) {
            println!("find {}({}) on {url}", hex_string(&peer.id), peer.addr);
            let find_peer = find_peer.clone();
            let task = async move {
                find_session(url.into(), find_peer).await?;
                sleep(Duration::from_secs(1)).await;
                Ok::<_, anyhow::Error>(())
            };
            tasks.spawn(task);
        }
    }
    while let Some(result) = tokio::select! {
        result = tasks.join_next() => result,
        result = peer_sessions.join_next() => return result.unwrap().map_err(Into::into).and_then(identity),
    } {
        result??
    }

    let url = urls[3];
    println!("boostrap peer on {url}");
    let config = BootstrapConfig {
        seed_peer: peers[0].clone(),
        host: [127, 0, 0, 1].into(),
    };
    let boostrap_peer = client
        .post(format!("{url}/bootstrap-peer"))
        .timeout(Duration::from_secs(10))
        .json(&config)
        .send()
        .await?
        .error_for_status()?
        .json::<Peer>()
        .await?;
    println!(
        "peer {} {}",
        hex_string(&boostrap_peer.id),
        boostrap_peer.addr
    );
    peer_sessions.spawn(peer_session(url.into(), shutdown.clone()));

    // let find_peer = FindPeer {
    //     target: boostrap_peer.id,
    //     count: 1,
    // };
    // for url in urls {
    //     println!(
    //         "find {}({}) on {url}",
    //         hex_string(&boostrap_peer.id),
    //         boostrap_peer.addr
    //     );
    //     let task = async {
    //         find_session(url.into(), find_peer.clone()).await?;
    //         sleep(Duration::from_secs(1)).await;
    //         Ok::<_, anyhow::Error>(())
    //     };
    //     tokio::select! {
    //         result = task => result?,
    //         result = peer_sessions.join_next() => result.unwrap()??,
    //     }
    // }

    shutdown.cancel();
    while let Some(result) = peer_sessions.join_next().await {
        result??
    }
    Ok(())
}
