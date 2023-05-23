use bincode::Options;
use ed25519_dalek::{Keypair, PublicKey, SecretKey, Signer, Verifier};
use neat_core::{actor::State, app::FunctionalState, route::ReplicaTable};

use crate::replica::ToReplica;

pub type Signature = ed25519_dalek::Signature;

fn replica_id(message: &ToReplica, n: usize) -> u8 {
    match message {
        ToReplica::Request(_) | ToReplica::Timeout(_) => unimplemented!(),
        ToReplica::PrePrepare(message, _) => (message.view_num as usize % n) as u8,
        ToReplica::Prepare(message) => message.replica_id,
        ToReplica::Commit(message) => message.replica_id,
    }
}

pub struct Verify<S> {
    keys: Vec<PublicKey>,
    pub state: S,
}

impl<S> Verify<S> {
    pub fn new(route: &ReplicaTable, state: S) -> Self {
        let keys = (0..route.len())
            .map(|i| (&SecretKey::from_bytes(&route.identity(i as _)).unwrap()).into())
            .collect();
        Self { keys, state }
    }
}

// not implement as PureState because later may need to send out `ViewChange`
// to another state
impl<S> State<(ToReplica, Signature)> for Verify<S>
where
    S: State<ToReplica>,
{
    fn update(&mut self, message: (ToReplica, Signature)) {
        let (message, signature) = message;
        if self.keys[replica_id(&message, self.keys.len()) as usize]
            .verify(&bincode::options().serialize(&message).unwrap(), &signature)
            .is_ok()
        {
            // TODO archive neccessary messages
            self.state.update(message)
        } else {
            //
        }
    }
}

pub struct Sign {
    key: Keypair,
}

impl Sign {
    pub fn new(route: &ReplicaTable, id: u8) -> Self {
        let secret = SecretKey::from_bytes(&route.identity(id)).unwrap();
        Self {
            key: Keypair {
                public: (&secret).into(),
                secret,
            },
        }
    }
}

impl FunctionalState<ToReplica> for Sign {
    type Output<'output> = (ToReplica, Signature) where Self: 'output;

    fn update(&mut self, input: ToReplica) -> Self::Output<'_> {
        let signature = self
            .key
            .sign(&bincode::options().serialize(&input).unwrap());
        (input, signature)
    }
}
