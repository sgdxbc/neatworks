// notice: `App`-specific `impl Workload`s are in `app` module
// only `App`-agnostic combinators live here
// maybe not the most reasonable organization but makes enough sense to me

use std::{
    fmt::{Debug, Display},
    sync::{
        atomic::{AtomicU32, Ordering::SeqCst},
        Arc,
    },
    time::{Duration, Instant},
};

use crate::{
    event::{
        erased::{events::Init, OnEvent},
        OnTimer, SendEvent, Timer,
    },
    message::Payload,
};

pub trait Workload {
    type Attach;

    fn next_op(&mut self) -> anyhow::Result<Option<(Payload, Self::Attach)>>;

    fn on_result(&mut self, result: Payload, attach: Self::Attach) -> anyhow::Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Iter<I>(pub I);

impl<T: Iterator<Item = Payload>> Workload for Iter<T> {
    type Attach = ();

    fn next_op(&mut self) -> anyhow::Result<Option<(Payload, Self::Attach)>> {
        Ok(self.0.next().map(|op| (op, ())))
    }

    fn on_result(&mut self, _: Payload, (): Self::Attach) -> anyhow::Result<()> {
        Ok(())
    }
}

impl<I> From<Iter<I>> for () {
    fn from(_: Iter<I>) -> Self {}
}

// coupling workload generation and latency measurement may not be a good design
// generally speaking, there should be a concept of "transaction" that composed from one or more
// ops, and latency is mean to be measured against transactions
// current the transaction concept is skipped, maybe revisit the design later
#[derive(Debug, derive_more::Deref)]
pub struct OpLatency<W> {
    #[deref]
    inner: W,
    pub latencies: Vec<Duration>,
}

impl<W> OpLatency<W> {
    pub fn new(inner: W) -> Self {
        Self {
            inner,
            latencies: Default::default(),
        }
    }
}

impl<W> From<OpLatency<W>> for Vec<Duration> {
    fn from(value: OpLatency<W>) -> Self {
        value.latencies
    }
}

impl<W: Workload> Workload for OpLatency<W> {
    type Attach = (Instant, W::Attach);

    fn next_op(&mut self) -> anyhow::Result<Option<(Payload, Self::Attach)>> {
        let Some((op, attach)) = self.inner.next_op()? else {
            return Ok(None);
        };
        Ok(Some((op, (Instant::now(), attach))))
    }

    fn on_result(&mut self, result: Payload, (start, attach): Self::Attach) -> anyhow::Result<()> {
        self.latencies.push(start.elapsed());
        self.inner.on_result(result, attach)
    }
}

#[derive(Debug, Clone, derive_more::Deref)]
pub struct Recorded<W> {
    #[deref]
    inner: W,
    pub invocations: Vec<(Payload, Payload)>,
}

impl<W> From<W> for Recorded<W> {
    fn from(value: W) -> Self {
        Self {
            inner: value,
            invocations: Default::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DryRecorded<T> {
    inner: T,
    pub invocations: Vec<(Payload, Payload)>,
}

impl<W: Workload> Workload for Recorded<W> {
    type Attach = (Payload, W::Attach);

    fn next_op(&mut self) -> anyhow::Result<Option<(Payload, Self::Attach)>> {
        Ok(self
            .inner
            .next_op()?
            .map(|(op, attach)| (op.clone(), (op, attach))))
    }

    fn on_result(&mut self, result: Payload, (op, attach): Self::Attach) -> anyhow::Result<()> {
        self.invocations.push((op, result.clone()));
        self.inner.on_result(result, attach)
    }
}

impl<W: Into<T>, T> From<Recorded<W>> for DryRecorded<T> {
    fn from(value: Recorded<W>) -> Self {
        Self {
            inner: value.inner.into(),
            invocations: value.invocations,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Total<W> {
    inner: W,
    remain_count: Arc<AtomicU32>,
}

impl<W> Total<W> {
    pub fn new(inner: W, count: u32) -> Self {
        Self {
            inner,
            remain_count: Arc::new(AtomicU32::new(count)),
        }
    }
}

impl<W: Workload> Workload for Total<W> {
    type Attach = W::Attach;

    fn next_op(&mut self) -> anyhow::Result<Option<(Payload, Self::Attach)>> {
        let mut remain_count = self.remain_count.load(SeqCst);
        loop {
            if remain_count == 0 {
                return Ok(None);
            }
            match self.remain_count.compare_exchange_weak(
                remain_count,
                remain_count - 1,
                SeqCst,
                SeqCst,
            ) {
                Ok(_) => break,
                Err(count) => remain_count = count,
            }
        }
        self.inner.next_op()
    }

    fn on_result(&mut self, result: Payload, attach: Self::Attach) -> anyhow::Result<()> {
        self.inner.on_result(result, attach)
    }
}

// impl<W: Into<T>, T> From<Total<W>> for T {
//     fn from(value: Total<W>) -> Self {
//         value.inner.into()
//     }
// }

#[derive(Debug, Clone)]
pub struct Check<I> {
    inner: I,
    expected_result: Option<Payload>,
}

impl<I> Check<I> {
    pub fn new(inner: I) -> Self {
        Self {
            inner,
            expected_result: None,
        }
    }
}

#[derive(Debug)]
pub struct UnexpectedResult {
    pub expect: Payload,
    pub actual: Payload,
}

impl Display for UnexpectedResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for UnexpectedResult {}

impl<I: Iterator<Item = (Payload, Payload)>> Workload for Check<I> {
    type Attach = ();

    fn next_op(&mut self) -> anyhow::Result<Option<(Payload, Self::Attach)>> {
        let Some((op, expected_result)) = self.inner.next() else {
            return Ok(None);
        };
        let replaced = self.expected_result.replace(expected_result);
        if replaced.is_some() {
            anyhow::bail!("only support close loop")
        }
        Ok(Some((op, ())))
    }

    fn on_result(&mut self, result: Payload, (): Self::Attach) -> anyhow::Result<()> {
        let Some(expected_result) = self.expected_result.take() else {
            anyhow::bail!("missing invocation")
        };
        if result == expected_result {
            Ok(())
        } else {
            Err(UnexpectedResult {
                expect: expected_result,
                actual: result,
            })?
        }
    }
}

impl<I> From<Check<I>> for () {
    fn from(_: Check<I>) -> Self {}
}

#[derive(Debug, Clone)]
pub struct Invoke(pub Payload);

// newtype namespace may be desired after the type erasure migration
// the `u32` field was for client id, and becomes unused after remove multiple
// client support on `CloseLoop`
// too lazy to refactor it off
pub type InvokeOk = (u32, Payload);

pub struct CloseLoop<W: Workload, E> {
    pub sender: E,
    pub workload: W,
    workload_attach: Option<W::Attach>,
    pub stop: Option<CloseLoopStop>,
    pub done: bool,
}

type CloseLoopStop = Box<dyn FnOnce() -> anyhow::Result<()> + Send + Sync>;

impl<W: Workload, E> Debug for CloseLoop<W, E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloseLoop").finish_non_exhaustive()
    }
}

impl<W: Workload, E> CloseLoop<W, E> {
    pub fn new(sender: E, workload: W) -> Self {
        Self {
            sender,
            workload,
            workload_attach: None,
            stop: None,
            done: false,
        }
    }
}

impl<W: Workload + Clone, E: Clone> Clone for CloseLoop<W, E>
where
    W::Attach: Clone,
{
    fn clone(&self) -> Self {
        if self.stop.is_some() {
            panic!("cannot clone CloseLoop with some `stop`")
        }
        Self {
            sender: self.sender.clone(),
            workload: self.workload.clone(),
            workload_attach: self.workload_attach.clone(),
            stop: None,
            done: self.done,
        }
    }
}

impl<W: Workload, E: SendEvent<Invoke>> OnEvent<Init> for CloseLoop<W, E> {
    fn on_event(&mut self, Init: Init, _: &mut impl Timer) -> anyhow::Result<()> {
        let (op, attach) = self
            .workload
            .next_op()?
            .ok_or(anyhow::anyhow!("not enough op"))?;
        let replaced = self.workload_attach.replace(attach);
        if replaced.is_some() {
            anyhow::bail!("duplicated launch")
        }
        self.sender.send(Invoke(op))
    }
}

impl<W: Workload, E: SendEvent<Invoke>> OnEvent<InvokeOk> for CloseLoop<W, E> {
    fn on_event(&mut self, (_, result): InvokeOk, _: &mut impl Timer) -> anyhow::Result<()> {
        let Some(attach) = self.workload_attach.take() else {
            anyhow::bail!("missing workload attach")
        };
        self.workload.on_result(result, attach)?;
        if let Some((op, attach)) = self.workload.next_op()? {
            self.workload_attach.replace(attach);
            self.sender.send(Invoke(op))
        } else {
            self.done = true;
            if let Some(stop) = self.stop.take() {
                stop()?
            }
            Ok(())
        }
    }
}

impl<W: Workload, E> OnTimer for CloseLoop<W, E> {
    fn on_timer(&mut self, _: crate::event::TimerId, _: &mut impl Timer) -> anyhow::Result<()> {
        unreachable!()
    }
}

pub mod check {
    use super::{CloseLoop, Workload};

    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    pub struct DryCloseLoop<T>(T);

    impl<W: Workload + Into<T>, T, E> From<CloseLoop<W, E>> for DryCloseLoop<T> {
        fn from(value: CloseLoop<W, E>) -> Self {
            Self(value.workload.into())
        }
    }
}
