use crate::actor::{Filtered, SharedClone, State};

pub trait FunctionalState<Input> {
    type Output<'output>
    where
        Self: 'output;

    fn update(&mut self, input: Input) -> Self::Output<'_>;

    fn install<S>(self, state: S) -> Install<Self, S>
    where
        Self: Sized,
    {
        Install(self, state)
    }

    fn install_filtered<S>(self, state: S) -> Install<Self, Filtered<S>>
    where
        Self: Sized,
    {
        Install(self, Filtered(state))
    }

    fn lift<L>(self, lift: L) -> Lift<Self, L>
    where
        Self: Sized,
    {
        Lift(self, lift)
    }

    fn lift_default<L>(self) -> Lift<Self, L>
    where
        Self: Sized,
        L: Default,
    {
        Lift(self, Default::default())
    }
}

#[derive(Debug, Clone)]
pub struct Closure<F>(pub F);

impl<F> From<F> for Closure<F> {
    fn from(value: F) -> Self {
        Self(value)
    }
}

impl<F, I, O> FunctionalState<I> for Closure<F>
where
    F: FnMut(I) -> O,
{
    // how to connection `'o` with `I`'s lifetime (if there's any)?
    type Output<'o> = O where Self: 'o;

    fn update(&mut self, input: I) -> Self::Output<'_> {
        (self.0)(input)
    }
}

// cannot have `F: Fn(I) -> O` bound here any more
// is this ok?
impl<F> SharedClone for Closure<F> where F: Clone {}

pub trait App {
    fn update(&mut self, op_num: u32, op: &[u8]) -> Vec<u8>;
}

pub type Message<'m> = (u32, &'m [u8]);

impl<A: App> FunctionalState<Message<'_>> for A {
    type Output<'o> = Vec<u8> where A: 'o;

    fn update(&mut self, input: Message<'_>) -> Self::Output<'_> {
        let (op_num, op) = input;
        self.update(op_num, op)
    }
}

// the name is too abstract...
#[derive(Debug, Clone)]
pub struct Install<A, S>(pub A, pub S);

impl<M, A, S> State<M> for Install<A, S>
where
    A: FunctionalState<M>,
    S: for<'o> State<A::Output<'o>>,
{
    fn update(&mut self, message: M) {
        self.1.update(self.0.update(message))
    }
}

impl<A, S> SharedClone for Install<A, S>
where
    A: SharedClone,
    S: SharedClone,
{
}

pub struct Inspect<S>(pub S);

impl<M, S> FunctionalState<M> for Inspect<S>
where
    S: State<M>,
    M: Clone,
{
    // is it expected to discard lifetime?
    type Output<'output> = M where Self: 'output;

    fn update(&mut self, input: M) -> Self::Output<'_> {
        self.0.update(input.clone());
        input
    }
}

pub struct Lift<S, L>(pub S, pub L);

impl<S, L, M> FunctionalState<M> for Lift<S, L>
where
    L: crate::message::Lift<S, M>,
{
    type Output<'o> = L::Out<'o> where Self: 'o;

    fn update(&mut self, input: M) -> Self::Output<'_> {
        self.1.update(&mut self.0, input)
    }
}
