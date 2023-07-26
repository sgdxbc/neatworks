use std::{collections::HashMap, hash::Hash, marker::PhantomData, time::Duration};

use neat_core::{message::Timeout, State};
use tokio::{
    select, spawn,
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    task::JoinHandle,
    time::{sleep, Instant},
};

#[derive(Debug)]
struct Sleeper<T> {
    reset: UnboundedSender<()>,
    task: JoinHandle<()>,
    _timeout: PhantomData<T>,
}

impl<T> Sleeper<T> {
    fn spawn(duration: Duration, wake: UnboundedSender<T>, timeout: T) -> Self
    where
        T: Send + 'static,
    {
        let reset = unbounded_channel();
        Self {
            reset: reset.0,
            task: spawn(Self::run(duration, reset.1, wake, timeout)),
            _timeout: PhantomData,
        }
    }

    async fn run(
        duration: Duration,
        mut reset: UnboundedReceiver<()>,
        wake: UnboundedSender<T>,
        timeout: T,
    ) {
        let sleep = sleep(duration);
        tokio::pin!(sleep);
        loop {
            select! {
                _ = &mut sleep => break,
                result = reset.recv() => if result.is_some() {
                    sleep.as_mut().reset(Instant::now() + duration);
                } else {
                    return; // unset timeout cause `reset` sender get dropped
                }
            }
        }
        if wake.send(timeout).is_err() {
            //
        }
    }
}

impl<T> Drop for Sleeper<T> {
    fn drop(&mut self) {
        self.task.abort()
    }
}

struct Reset;

impl<T> State<Reset> for Sleeper<T> {
    fn update(&mut self, Reset: Reset) {
        if self.reset.send(()).is_err() {
            //
        }
    }
}

// because both consuming and producing `Timeout<T>` tightly couples with the
// `sleepers` state, it should be better to not split `Control` into two parts
#[derive(Debug)]
pub struct Control<T> {
    wake: (UnboundedSender<T>, UnboundedReceiver<T>),
    sleepers: HashMap<T, Sleeper<T>>,
}

impl<T> Default for Control<T> {
    fn default() -> Self {
        Self {
            wake: unbounded_channel(),
            sleepers: Default::default(),
        }
    }
}

impl<T> State<Timeout<T>> for Control<T>
where
    T: Hash + Eq + Clone + Send + 'static,
{
    // TODO better mistake detection
    fn update(&mut self, message: Timeout<T>) {
        use Timeout::*;
        match message {
            Set(timeout) => {
                self.sleepers.insert(
                    timeout.clone(),
                    // TODO
                    Sleeper::spawn(Duration::from_millis(100), self.wake.0.clone(), timeout),
                );
            }
            Reset(timeout) => {
                if let Some(sleeper) = self.sleepers.get_mut(&timeout) {
                    sleeper.update(crate::time::Reset)
                }
            }
            Unset(timeout) => {
                self.sleepers.remove(&timeout);
            }
        }
    }
}

impl<T> Control<T> {
    pub async fn recv(&mut self) -> T
    where
        T: Hash + Eq,
    {
        loop {
            let timeout = self
                .wake
                .1
                .recv()
                .await
                .expect("control own sender cannot be dropped");
            if self.sleepers.remove(&timeout).is_some() {
                return timeout;
            }
        }
    }
}
