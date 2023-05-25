use std::{marker::PhantomData, time::Duration};

use neat_core::{
    actor::State,
    app::FunctionalState,
    message::{Dispatch, Timeout},
};
use tokio::{
    select, spawn,
    task::JoinHandle,
    time::{sleep, Instant},
};

// Unset semantic is only guaranteed when message passing is fully synchronous
// that is, both `Sleeper` and (`Control`-installed) `Dispatch` must be directly
// embedded instead of `Drive`ed.
// in the future maybe a marker trait should be added to encode this requirement

#[derive(Debug)]
pub struct Sleeper<T> {
    reset: flume::Sender<()>,
    task: JoinHandle<()>,
    _timeout: PhantomData<T>,
}

impl<T> Sleeper<T> {
    fn spawn(duration: Duration, wake: flume::Sender<T>, timeout: T) -> Self
    where
        T: Send + 'static,
    {
        let reset = flume::unbounded();
        Self {
            reset: reset.0,
            task: spawn(Self::run(duration, reset.1, wake, timeout)),
            _timeout: PhantomData,
        }
    }

    async fn run(
        duration: Duration,
        reset: flume::Receiver<()>,
        wake: flume::Sender<T>,
        timeout: T,
    ) {
        let sleep = sleep(duration);
        tokio::pin!(sleep);
        loop {
            select! {
                _ = &mut sleep => break,
                result = reset.recv_async() => if result.is_ok() {
                    sleep.as_mut().reset(Instant::now() + duration);
                } else {
                    return; // unset timeout cause `reset` sender get dropped
                }
            }
        }
        // any further reset is ignored, which should be fine
        // a rendezvous `wake` channel is used to ensure that unset timeout is
        // never waked, hope it works as i expect
        if wake.send_async(timeout).await.is_err() {
            //
        }
    }
}

impl<T> Drop for Sleeper<T> {
    fn drop(&mut self) {
        self.task.abort()
    }
}

pub struct Reset;

impl<T> State<Reset> for Sleeper<T> {
    fn update(&mut self, Reset: Reset) {
        if self.reset.send(()).is_err() {
            //
        }
    }
}

#[derive(Debug, Clone)]
pub struct Waker<T, D> {
    wake: flume::Receiver<T>,
    // alternative: decouple with `dispatch`, ask user to `Inspect` timeout and
    // do removal by themselves
    // sounds like unnecessary exposure of internal details
    pub dispatch: D,
}

impl<T, D> Waker<T, D> {
    pub async fn recv<S, M>(&mut self) -> Option<T>
    where
        D: State<Dispatch<T, S, M>>,
        T: Clone,
    {
        if let Ok(timeout) = self.wake.recv_async().await {
            self.dispatch.update(Dispatch::Remove(timeout.clone()));
            Some(timeout)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone)]
pub struct Control<T>(flume::Sender<T>);

pub fn new<T, D>(dispatch: D) -> (Waker<T, D>, Control<T>) {
    let wake = flume::bounded(0);
    (
        Waker {
            wake: wake.1,
            dispatch,
        },
        Control(wake.0),
    )
}

impl<T> FunctionalState<Timeout<T>> for Control<T>
where
    T: Clone + Send + 'static,
{
    type Output<'output> = Dispatch<T, Sleeper<T>, Reset> where Self: 'output;

    fn update(&mut self, input: Timeout<T>) -> Self::Output<'_> {
        use {Dispatch::*, Timeout::*};
        match input {
            Set(timeout) => Insert(
                timeout.clone(),
                Sleeper::spawn(Default::default(), self.0.clone(), timeout),
            ),
            Reset(timeout) => Update(timeout, crate::time::Reset),
            Unset(timeout) => Remove(timeout),
        }
    }
}
