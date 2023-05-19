use std::marker::PhantomData;

use bincode::Options;
use serde::{de::DeserializeOwned, Serialize};
use wm_core::{
    actor::SharedClone,
    app::{Closure, PureState},
};

struct De<M>(PhantomData<M>);

impl<M> Clone for De<M> {
    fn clone(&self) -> Self {
        Self(PhantomData)
    }
}

impl<M> Copy for De<M> {}

impl<M> SharedClone for De<M> {}

impl<'i, M> PureState<'i> for De<M>
where
    M: DeserializeOwned,
{
    type Input = &'i [u8];
    type Output<'output> = M where Self: 'output;

    fn update(&mut self, input: Self::Input) -> Self::Output<'_> {
        bincode::options()
            //
            .allow_trailing_bytes()
            .deserialize(input)
            .unwrap()
    }
}

pub const fn de<M>(
) -> impl for<'i, 'o> PureState<'i, Input = &'i [u8], Output<'o> = M> + SharedClone
where
    M: DeserializeOwned + 'static,
{
    De(PhantomData)
}

pub fn ser<M>() -> impl for<'i, 'o> PureState<'i, Input = M, Output<'o> = Vec<u8>>
where
    M: Serialize + 'static,
{
    Closure::new(|message| bincode::options().serialize(&message).unwrap())
}
