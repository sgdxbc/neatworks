use std::{marker::PhantomData, time::Duration};

use murmesh_core::{actor::State, app::FunctionalState, dispatch, timeout};
use tokio::{
    select, spawn,
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    time::{sleep, Instant},
};

// Unset semantic is only guaranteed when message passing is fully synchronous
// that is, both `Sleeper` and (`Control`-installed) `Dispatch` must be directly
// embedded instead of `Drive`ed.
// in the future maybe a marker trait should be added to encode this requirement

#[derive(Debug)]
pub struct Sleeper<T> {
    reset: UnboundedSender<()>,
    _timeout: PhantomData<T>,
}

impl<T> Sleeper<T> {
    fn spawn(duration: Duration, wake: flume::Sender<T>, timeout: T) -> Self
    where
        T: Send + 'static,
    {
        let reset = unbounded_channel();
        spawn(Self::run(duration, reset.1, wake, timeout));
        Self {
            reset: reset.0,
            _timeout: PhantomData,
        }
    }

    async fn run(
        duration: Duration,
        mut reset: UnboundedReceiver<()>,
        wake: flume::Sender<T>,
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
        // any further reset is ignored, which should be fine
        // a rendezvous `wake` channel is used to ensure that unset timeout is
        // never waked, hope it works as i expect
        if wake.send_async(timeout).await.is_err() {
            //
        }
    }
}

pub struct Reset;

impl<T> State<'_> for Sleeper<T> {
    type Message = Reset;

    fn update(&mut self, Reset: Self::Message) {
        if self.reset.send(()).is_err() {
            //
        }
    }
}

#[derive(Debug, Clone)]
pub struct Waker<T, S> {
    wake: flume::Receiver<T>,
    pub state: S,
}

impl<T, S> Waker<T, S> {
    pub async fn start(&mut self)
    where
        S: for<'m> State<'m, Message = T>,
    {
        while let Ok(timeout) = self.wake.recv_async().await {
            self.state.update(timeout)
        }
    }
}

#[derive(Debug, Clone)]
pub struct Control<T>(flume::Sender<T>);

// can we turn this into a `From` impl?
pub fn new<S, T>(state: S) -> (Waker<T, S>, Control<T>) {
    let wake = flume::bounded(0);
    (
        Waker {
            wake: wake.1,
            state,
        },
        Control(wake.0),
    )
}

impl<T> FunctionalState<'_> for Control<T>
where
    T: Clone + Send + 'static,
{
    type Input = timeout::Message<T>;
    type Output<'output> = dispatch::Message<T, Sleeper<T>, Reset> where Self: 'output;

    fn update(&mut self, input: Self::Input) -> Self::Output<'_> {
        use {dispatch::Message::*, timeout::Message::*};
        match input {
            Set(timeout) => Insert(
                timeout.clone(),
                Sleeper::spawn(Default::default(), self.0.clone(), timeout),
            ),
            Reset(timeout) => Update(timeout, crate::Reset),
            Unset(timeout) => Remove(timeout),
        }
    }
}
