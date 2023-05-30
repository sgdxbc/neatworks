use bincode::Options;
use ed25519_dalek::{Keypair, PublicKey, SecretKey, Signer, Verifier};
use neat_core::{route::ReplicaTable, FunctionalState};

use crate::replica::FromReplica;

pub type Signature = ed25519_dalek::Signature;

fn replica_id(message: &FromReplica, n: usize) -> u8 {
    match message {
        FromReplica::PrePrepare(message, _) => (message.view_num as usize % n) as u8,
        FromReplica::Prepare(message) => message.replica_id,
        FromReplica::Commit(message) => message.replica_id,
    }
}

pub struct Verify {
    keys: Vec<PublicKey>,
}

impl Verify {
    pub fn new(route: &ReplicaTable) -> Self {
        let keys = (0..route.len())
            .map(|i| (&SecretKey::from_bytes(&route.identity(i as _)).unwrap()).into())
            .collect();
        Self { keys }
    }
}

impl FunctionalState<(FromReplica, Signature)> for Verify {
    type Output<'output> = Option<(FromReplica, Signature)> where Self: 'output;

    fn update(&mut self, message: (FromReplica, Signature)) -> Self::Output<'_> {
        let (message, signature) = message;
        self.keys[replica_id(&message, self.keys.len()) as usize]
            .verify(&bincode::options().serialize(&message).unwrap(), &signature)
            .ok()
            .map(|()| (message, signature))
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

impl FunctionalState<FromReplica> for Sign {
    type Output<'output> = (FromReplica, Signature) where Self: 'output;

    fn update(&mut self, input: FromReplica) -> Self::Output<'_> {
        let signature = self
            .key
            .sign(&bincode::options().serialize(&input).unwrap());
        (input, signature)
    }
}
