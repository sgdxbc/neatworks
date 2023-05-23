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

#[derive(Debug)]
pub enum Message<K, S, M> {
    Insert(K, S),
    Update(K, M),
    Remove(K),
}

impl<K, S, M> From<(K, M)> for Message<K, S, M> {
    fn from(value: (K, M)) -> Self {
        Self::Update(value.0, value.1)
    }
}

impl<'m, K, S, M> State<'m> for Dispatch<K, S>
where
    S: State<'m, Message = M>,
    K: Eq + Hash,
{
    type Message = Message<K, S, M>;

    fn update(&mut self, message: Self::Message) {
        match message {
            Message::Insert(k, state) => {
                let evicted = self.insert_state(k, state);
                if evicted.is_some() {
                    //
                }
            }
            Message::Update(k, message) => {
                if let Some(state) = self.states.get_mut(&k) {
                    state.update(message)
                } else {
                    //
                }
            }
            Message::Remove(k) => {
                if self.states.remove(&k).is_none() {
                    //
                }
            }
        }
    }
}
