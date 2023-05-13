use std::marker::PhantomData;

use crate::actor::{Filtered, SharedClone, State};

pub trait PureState<'input> {
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

impl<F, I, O> PureState<'_> for Closure<F, I, O>
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

impl<'i, A: App> PureState<'i> for A {
    type Input = (u32, &'i [u8]);
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
    A: PureState<'i>,
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
