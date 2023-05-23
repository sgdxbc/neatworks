use bincode::Options;
use neat_core::{
    actor::SharedClone,
    app::{Closure, FunctionalState},
};
use serde::{de::DeserializeOwned, Serialize};

pub fn de<M>() -> impl for<'i, 'o> FunctionalState<&'i [u8], Output<'o> = M> + SharedClone
where
    M: DeserializeOwned + 'static,
{
    Closure(|message: &_| {
        bincode::options()
            //
            .allow_trailing_bytes()
            .deserialize(message)
            .unwrap()
    })
}

pub fn ser<M>() -> impl for<'o> FunctionalState<M, Output<'o> = Vec<u8>> + SharedClone
where
    M: Serialize + 'static,
{
    Closure(|message| bincode::options().serialize(&message).unwrap())
}
