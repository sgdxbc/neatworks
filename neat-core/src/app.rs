use std::marker::PhantomData;

use crate::actor::{Filtered, SharedClone, State};

pub trait FunctionalState<'input> {
    type Input;
    type Output<'output>
    where
        Self: 'output;

    fn update(&mut self, input: Self::Input) -> Self::Output<'_>;

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

    fn lift<M>(self) -> Lift<Self, M>
    where
        Self: Sized,
    {
        Lift(self, PhantomData)
    }
}

pub struct Closure<F, I, O>(F, PhantomData<(I, O)>);

impl<F, I, O> Closure<F, I, O> {
    pub const fn new(f: F) -> Self {
        Self(f, PhantomData)
    }
}

impl<F, I, O> From<F> for Closure<F, I, O> {
    fn from(value: F) -> Self {
        Self::new(value)
    }
}

impl<F, I, O> FunctionalState<'_> for Closure<F, I, O>
where
    F: FnMut(I) -> O,
{
    type Input = I;
    type Output<'o> = O where Self: 'o;

    fn update(&mut self, input: Self::Input) -> Self::Output<'_> {
        (self.0)(input)
    }
}

impl<F, I, O> Clone for Closure<F, I, O>
where
    F: Clone,
{
    fn clone(&self) -> Self {
        Self(self.0.clone(), PhantomData)
    }
}

impl<F, I, O> SharedClone for Closure<F, I, O> where F: Fn(I) -> O + Clone {}

pub trait App {
    fn update(&mut self, op_num: u32, op: &[u8]) -> Vec<u8>;
}

pub type Message<'m> = (u32, &'m [u8]);

impl<'i, A: App> FunctionalState<'i> for A {
    type Input = Message<'i>;
    type Output<'o> = Vec<u8> where A: 'o;

    fn update(&mut self, input: Self::Input) -> Self::Output<'_> {
        let (op_num, op) = input;
        self.update(op_num, op)
    }
}

// the name is too abstract...
#[derive(Debug, Clone)]
pub struct Install<A, S>(pub A, pub S);

impl<'i, A, S> State<'i> for Install<A, S>
where
    A: FunctionalState<'i>,
    S: for<'o> State<'o, Message = A::Output<'o>>,
{
    type Message = A::Input;

    fn update(&mut self, message: Self::Message) {
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

impl<'m, S> FunctionalState<'m> for Inspect<S>
where
    S: State<'m>,
    S::Message: Clone,
{
    type Input = S::Message;
    // probably need to adjust
    type Output<'output> = S::Message where Self: 'output;

    fn update(&mut self, input: Self::Input) -> Self::Output<'_> {
        self.0.update(input.clone());
        input
    }
}

pub struct Lift<S, M>(pub S, PhantomData<M>);

impl<'m, S, M> FunctionalState<'m> for Lift<S, M>
where
    S: FunctionalState<'m>,
    M: crate::message::Lift<'m, S::Input, S>,
{
    type Input = M;
    type Output<'o> = M::Out<'o> where Self: 'o;

    fn update(&mut self, input: Self::Input) -> Self::Output<'_> {
        input.update(&mut self.0)
    }
}
