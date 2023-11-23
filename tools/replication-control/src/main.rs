use std::{sync::OnceLock, time::Duration};

use messages::AddrBook;
use replication_control_messages as messages;
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
    let result = benchmark_session().await?;
    println!("{result:?}");
    Ok(())
}

static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

async fn client_session(config: messages::Client, url: String) -> anyhow::Result<(f32, Duration)> {
    let client = CLIENT.get().unwrap();
    client
        .post(format!("{url}/run-client"))
        .json(&config)
        .send()
        .await?
        .error_for_status()?;
    loop {
        sleep(Duration::from_secs(1)).await;
        let result = client
            .post(format!("{url}/take-client-result"))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if let Some(result) = result {
            break Ok(result);
        }
    }
}

async fn replica_session(
    config: messages::Replica,
    url: String,
    shutdown: CancellationToken,
) -> anyhow::Result<()> {
    let client = CLIENT.get().unwrap();
    client
        .post(format!("{url}/run-replica"))
        .json(&config)
        .send()
        .await?
        .error_for_status()?;
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
    client
        .post(format!("{url}/reset-replica"))
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

async fn benchmark_session() -> anyhow::Result<(f32, Duration)> {
    let mut addr_book = AddrBook::default();
    addr_book
        .client_addrs
        .insert(0, ([127, 0, 0, 1], 20000).into());
    addr_book
        .replica_addrs
        .insert(0, ([127, 0, 0, 1], 30000).into());
    let mut replica_sessions = JoinSet::new();
    let shutdown = CancellationToken::new();
    let config = messages::Replica {
        id: 0,
        addr_book: addr_book.clone(),
    };
    let url = "http://127.0.0.1:10000".into();
    replica_sessions.spawn(replica_session(config, url, shutdown.clone()));
    let mut client_sessions = JoinSet::new();
    let config = messages::Client {
        id_range: 0..1,
        addr_book,
    };
    let url = "http://127.0.0.1:10001".into();
    client_sessions.spawn(client_session(config, url));
    let mut throughput_sum = 0.;
    let mut latency_sum = Duration::ZERO;
    while let Some(client_result) = tokio::select! {
        result = client_sessions.join_next() => result,
        result = replica_sessions.join_next() => Err(result.unwrap().unwrap_err())?,
    } {
        let (throughput, latency) = client_result??;
        throughput_sum += throughput;
        latency_sum += latency * throughput as _;
    }
    shutdown.cancel();
    while let Some(replica_result) = replica_sessions.join_next().await {
        replica_result??
    }
    Ok((throughput_sum, latency_sum / throughput_sum as _))
}
