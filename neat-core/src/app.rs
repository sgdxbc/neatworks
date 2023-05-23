use std::marker::PhantomData;

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

impl<F, I, O> FunctionalState<I> for Closure<F, I, O>
where
    F: FnMut(I) -> O,
{
    // this does not give up any flexibility since closure can never return
    // reference of captured objects
    type Output<'o> = O where Self: 'o;

    fn update(&mut self, input: I) -> Self::Output<'_> {
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

pub struct Lift<S, M>(pub S, PhantomData<M>);

// impl<S, M> FunctionalState<M> for Lift<S, M>
// where
//     M: crate::message::Lift<'m, S::Input, S>,
// {
//     type Output<'o> = M::Out<'o> where Self: 'o;

//     fn update(&mut self, input: M) -> Self::Output<'_> {
//         input.update(&mut self.0)
//     }
// }
