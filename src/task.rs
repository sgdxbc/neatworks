use std::future::Future;

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub struct BackgroundSpawner {
    err_sender: UnboundedSender<crate::Error>,
    token: CancellationToken,
}

impl BackgroundSpawner {
    pub fn spawn(&self, task: impl Future<Output = crate::Result<()>> + Send + 'static) {
        let Self { err_sender, token } = self.clone();
        tokio::spawn(async move {
            let result = tokio::select! {
                result = tokio::spawn(task) => result,
                _ = token.cancelled() => return,
            };
            if let Err(err) = result
                .map_err(|err| crate::err!(err))
                .and_then(std::convert::identity)
            {
                err_sender
                    .send(err)
                    .expect("background monitor not shutdown")
            }
        });
    }
}

#[derive(Debug)]
pub struct BackgroundMonitor(UnboundedReceiver<crate::Error>);

impl BackgroundMonitor {
    pub async fn wait(&mut self) -> crate::Result<()> {
        self.0.recv().await.map(Err).unwrap_or(Ok(()))
    }
}
