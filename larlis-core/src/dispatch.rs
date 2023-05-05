use std::{collections::HashMap, hash::Hash};

use crate::actor::State;

pub struct Dispatch<K, S> {
    states: HashMap<K, S>,
}

impl<K, S> Default for Dispatch<K, S> {
    fn default() -> Self {
        Self {
            states: Default::default(),
        }
    }
}

impl<K, S> Dispatch<K, S> {
    pub fn insert_state(&mut self, k: K, state: S) -> Option<S>
    where
        K: Eq + Hash,
    {
        self.states.insert(k, state)
    }
}

impl<'m, K, S, M> State<'m> for Dispatch<K, S>
where
    S: State<'m, Message = M>,
    K: Eq + Hash,
{
    type Message = (K, M);

    fn update(&mut self, message: Self::Message) {
        if let Some(state) = self.states.get_mut(&message.0) {
            state.update(message.1)
        }
    }
}
