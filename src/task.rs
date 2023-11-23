use std::future::Future;

use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub struct BackgroundSpawner {
    err_sender: UnboundedSender<crate::Error>,
    token: CancellationToken,
}

impl BackgroundSpawner {
    pub fn spawn(&self, task: impl Future<Output = crate::Result<()>> + Send + 'static) {
        let err_sender = self.err_sender.clone();
        let token = self.token.clone();
        let mut task = tokio::spawn(task);
        tokio::spawn(async move {
            let result = tokio::select! {
                result = &mut task => result,
                () = token.cancelled() => {
                    task.abort();
                    task.await
                }
            };
            let err = match result {
                Err(err) if !err.is_cancelled() => err.into(),
                Ok(Err(err)) => err,
                _ => return,
            };
            err_sender
                .send(err)
                .expect("background monitor not shutdown")
        });
    }
}

#[derive(Debug)]
pub struct BackgroundMonitor {
    err_sender: UnboundedSender<crate::Error>,
    err_receiver: UnboundedReceiver<crate::Error>,
    token: CancellationToken,
}

impl Default for BackgroundMonitor {
    fn default() -> Self {
        let (err_sender, err_receiver) = unbounded_channel();
        Self {
            err_sender,
            err_receiver,
            token: CancellationToken::new(),
        }
    }
}

impl BackgroundMonitor {
    pub fn spawner(&self) -> BackgroundSpawner {
        BackgroundSpawner {
            err_sender: self.err_sender.clone(),
            token: self.token.clone(),
        }
    }

    pub async fn wait(&mut self) -> crate::Result<()> {
        match self.err_receiver.recv().await {
            Some(err) => {
                self.token.cancel();
                Err(err)
            }
            None => Ok(()),
        }
    }

    // pub fn cancel(&self) {
    //     self.token.cancel()
    // }
}
