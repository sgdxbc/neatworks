use std::marker::PhantomData;

use bincode::Options;
use neat_core::{
    actor::SharedClone,
    app::{Closure, FunctionalState},
};
use serde::{de::DeserializeOwned, Serialize};

struct De<M>(PhantomData<M>);

impl<M> Clone for De<M> {
    fn clone(&self) -> Self {
        Self(PhantomData)
    }
}

impl<M> Copy for De<M> {}

impl<M> SharedClone for De<M> {}

impl<M> FunctionalState<&'_ [u8]> for De<M>
where
    M: DeserializeOwned,
{
    type Output<'output> = M where Self: 'output;

    fn update(&mut self, input: &[u8]) -> Self::Output<'_> {
        bincode::options()
            //
            .allow_trailing_bytes()
            .deserialize(input)
            .unwrap()
    }
}

pub fn de<M>() -> impl for<'i, 'o> FunctionalState<&'i [u8], Output<'o> = M> + SharedClone
where
    M: DeserializeOwned + 'static,
{
    De(PhantomData)
}

pub fn ser<M>() -> impl for<'o> FunctionalState<M, Output<'o> = Vec<u8>> + SharedClone
where
    M: Serialize + 'static,
{
    Closure::new(|message| bincode::options().serialize(&message).unwrap())
}
