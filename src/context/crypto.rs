use std::{collections::HashMap, hash::Hash, mem::take, sync::Arc};

use ed25519_dalek::Sha512;
use hmac::{Hmac, Mac};
use k256::{
    schnorr::signature::{DigestSigner, DigestVerifier},
    sha2::{Digest, Sha256},
};
use serde::{Deserialize, Serialize};

use super::{
    ordered_multicast::{OrderedMulticast, Variant},
    ReplicaIndex,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signed<M> {
    pub inner: M,
    pub signature: Signature,
}

impl<M> std::ops::Deref for Signed<M> {
    type Target = M;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Signature {
    Plain,
    SimulatedPrivate,
    SimulatedPublic,
    K256(k256::ecdsa::Signature),
    Ed25519(ed25519_dalek::Signature),
    Hmac([u8; 32]),
}

impl<M: DigestHash> Hash for Signed<M> {
    fn hash<H>(&self, hasher: &mut H)
    where
        H: std::hash::Hasher,
    {
        self.inner.hash(hasher);
        match &self.signature {
            Signature::Plain | Signature::SimulatedPrivate | Signature::SimulatedPublic => {} // TODO
            Signature::K256(signature) => hasher.write(&signature.to_bytes()),
            Signature::Ed25519(signature) => hasher.write(&signature.to_bytes()),
            Signature::Hmac(codes) => hasher.write(codes),
        }
    }
}

pub enum Hasher {
    Sha256(Sha256),
    Sha512(Sha512),
    Hmac(Hmac<Sha256>),
    Bytes(Vec<u8>),
}

impl Hasher {
    pub fn update(&mut self, data: impl AsRef<[u8]>) {
        match self {
            Self::Sha256(hasher) => hasher.update(data),
            Self::Sha512(hasher) => hasher.update(data),
            Self::Hmac(hasher) => hasher.update(data.as_ref()),
            Self::Bytes(hasher) => hasher.extend(data.as_ref()),
        }
    }

    pub fn chain_update(self, data: impl AsRef<[u8]>) -> Self {
        match self {
            Self::Sha256(hasher) => Self::Sha256(hasher.chain_update(data)),
            Self::Sha512(hasher) => Self::Sha512(hasher.chain_update(data)),
            Self::Hmac(hasher) => Self::Hmac(hasher.chain_update(data)),
            Self::Bytes(hasher) => Self::Bytes([&hasher, data.as_ref()].concat()),
        }
    }
}

impl std::hash::Hasher for Hasher {
    fn write(&mut self, buf: &[u8]) {
        self.update(buf)
    }

    fn finish(&self) -> u64 {
        unimplemented!()
    }
}

pub trait DigestHash {
    fn hash(&self, hasher: &mut impl std::hash::Hasher);
}

impl<T: Hash> DigestHash for T {
    fn hash(&self, hasher: &mut impl std::hash::Hasher) {
        Hash::hash(self, hasher)
    }
}

impl Hasher {
    pub fn sha256(message: &impl DigestHash) -> Sha256 {
        let mut digest = Sha256::new();
        Self::sha256_update(message, &mut digest);
        digest
    }

    pub fn sha256_update(message: &impl DigestHash, digest: &mut Sha256) {
        let mut hasher = Self::Sha256(digest.clone());
        message.hash(&mut hasher);
        if let Self::Sha256(new_digest) = hasher {
            *digest = new_digest
        } else {
            unreachable!()
        };
    }

    pub fn sha512(message: &impl DigestHash) -> Sha512 {
        let mut digest = Sha512::new();
        Self::sha512_update(message, &mut digest);
        digest
    }

    pub fn sha512_update(message: &impl DigestHash, digest: &mut Sha512) {
        let mut hasher = Self::Sha512(digest.clone());
        message.hash(&mut hasher);
        if let Self::Sha512(new_digest) = hasher {
            *digest = new_digest
        } else {
            unreachable!()
        };
    }

    pub fn hmac_update(message: &impl DigestHash, hmac: &mut Hmac<Sha256>) {
        let mut hasher = Self::Hmac(hmac.clone());
        message.hash(&mut hasher);
        if let Self::Hmac(new_hmac) = hasher {
            *hmac = new_hmac
        } else {
            unreachable!()
        };
    }

    pub fn bytes(message: &impl DigestHash) -> Vec<u8> {
        let mut buf = Vec::new();
        Self::bytes_update(message, &mut buf);
        buf
    }

    pub fn bytes_update(message: &impl DigestHash, buf: &mut Vec<u8>) {
        let mut hasher = Self::Bytes(take(buf));
        message.hash(&mut hasher);
        if let Self::Bytes(new_buf) = hasher {
            *buf = new_buf
        } else {
            unreachable!()
        };
    }
}

#[derive(Debug, Clone)]
pub enum Signer {
    Simulated,
    Standard(Box<StandardSigner>),
}

#[derive(Debug, Clone)]
pub struct StandardSigner {
    signing_key: Option<SigningKey>,
    hmac: Hmac<Sha256>,
}

#[derive(Debug, Clone)]
pub enum SigningKey {
    K256(k256::ecdsa::SigningKey),
    Ed25519(ed25519_dalek::SigningKey),
}

pub fn hardcoded_k256(index: ReplicaIndex) -> SigningKey {
    let k = format!("hardcoded-{index}");
    let mut buf = [0; 32];
    buf[..k.as_bytes().len()].copy_from_slice(k.as_bytes());
    SigningKey::K256(k256::ecdsa::SigningKey::from_slice(&buf).unwrap())
}

pub fn hardcoded_hmac() -> Hmac<Sha256> {
    Hmac::new_from_slice("shared".as_bytes()).unwrap()
}

impl Signer {
    pub fn new_standard(signing_key: impl Into<Option<SigningKey>>) -> Self {
        Self::Standard(Box::new(StandardSigner {
            signing_key: signing_key.into(),
            hmac: hardcoded_hmac(),
        }))
    }

    pub fn sign_public<M>(&self, message: M) -> Signed<M>
    where
        M: DigestHash,
    {
        match self {
            Self::Simulated => Signed {
                inner: message,
                signature: Signature::SimulatedPublic,
            },
            Self::Standard(signer) => signer.sign_public(message),
        }
    }

    pub fn sign_private<M>(&self, message: M) -> Signed<M>
    where
        M: DigestHash,
    {
        match self {
            Self::Simulated => Signed {
                inner: message,
                signature: Signature::SimulatedPrivate,
            },
            Self::Standard(signer) => signer.sign_private(message),
        }
    }
}

impl StandardSigner {
    fn sign_public<M>(&self, message: M) -> Signed<M>
    where
        M: DigestHash,
    {
        let signature = match self.signing_key.as_ref().unwrap() {
            SigningKey::K256(signing_key) => {
                let digest = Hasher::sha256(&message);
                Signature::K256(signing_key.sign_digest(digest))
            }
            SigningKey::Ed25519(signing_key) => {
                let digest = Hasher::sha512(&message);
                Signature::Ed25519(signing_key.sign_digest(digest))
            }
        };
        Signed {
            inner: message,
            signature,
        }
    }

    fn sign_private<M>(&self, message: M) -> Signed<M>
    where
        M: DigestHash,
    {
        let mut hmac = self.hmac.clone();
        Hasher::hmac_update(&message, &mut hmac);
        Signed {
            signature: Signature::Hmac(hmac.finalize().into_bytes().into()),
            inner: message,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Verifier<I> {
    Nop,
    Simulated,
    Standard(Box<StandardVerifier<I>>),
}

#[derive(Debug, Clone)]
pub struct StandardVerifier<I> {
    verifying_keys: HashMap<I, VerifyingKey>,
    hmac: Hmac<Sha256>,
    variant: Arc<Variant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyingKey {
    K256(k256::ecdsa::VerifyingKey),
    Ed25519(ed25519_dalek::VerifyingKey),
}

impl SigningKey {
    pub fn verifying_key(&self) -> VerifyingKey {
        match self {
            Self::K256(signing_key) => VerifyingKey::K256(*signing_key.verifying_key()),
            Self::Ed25519(signing_key) => VerifyingKey::Ed25519(signing_key.verifying_key()),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Invalid {
    Public,
    Private,
    Variant,
}

impl std::fmt::Display for Invalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for Invalid {}

impl<I> Verifier<I> {
    pub fn new_standard(variant: impl Into<Arc<Variant>>) -> Self {
        Self::Standard(Box::new(StandardVerifier {
            verifying_keys: Default::default(),
            hmac: hardcoded_hmac(),
            variant: variant.into(),
        }))
    }

    pub fn insert_verifying_key(&mut self, identity: I, verifying_key: VerifyingKey)
    where
        I: Hash + Eq,
    {
        match self {
            Self::Nop => {}
            Self::Simulated => unimplemented!(),
            Self::Standard(verifier) => {
                let evicted = verifier.verifying_keys.insert(identity, verifying_key);
                assert!(evicted.is_none())
            }
        }
    }

    pub fn verify<M>(
        &self,
        message: &Signed<M>,
        identity: impl Into<Option<I>>,
    ) -> Result<(), Invalid>
    where
        M: DigestHash,
        I: Hash + Eq,
    {
        match (self, &message.signature) {
            (Self::Nop, _) => Ok(()),
            (Self::Simulated, Signature::SimulatedPrivate | Signature::SimulatedPublic) => Ok(()),
            (Self::Simulated, _) => unimplemented!(),
            (Self::Standard(verifier), Signature::Hmac(code)) => {
                let mut hmac = verifier.hmac.clone();
                Hasher::hmac_update(message, &mut hmac);
                hmac.verify(code.into()).map_err(|_| Invalid::Private)
            }
            (Self::Standard(verifier), signature) => {
                match (
                    &verifier.verifying_keys[&identity.into().unwrap()],
                    signature,
                ) {
                    (VerifyingKey::K256(verifying_key), Signature::K256(signature)) => {
                        verifying_key
                            .verify_digest(Hasher::sha256(&**message), signature)
                            .map_err(|_| Invalid::Public)
                    }
                    (VerifyingKey::Ed25519(verifying_key), Signature::Ed25519(signature)) => {
                        verifying_key
                            .verify_digest(Hasher::sha512(&**message), signature)
                            .map_err(|_| Invalid::Public)
                    }
                    _ => Err(Invalid::Variant),
                }
            }
        }
    }

    pub fn verify_ordered_multicast<M>(&self, message: &OrderedMulticast<M>) -> Result<(), Invalid>
    where
        M: DigestHash,
    {
        match self {
            Self::Nop => Ok(()),
            Self::Simulated => unimplemented!(),
            Self::Standard(verifier) => verifier.variant.verify(message),
        }
    }
}

pub trait Sign<M> {
    fn sign(message: M, signer: &Signer) -> Self;
}

impl<M, N: Into<M>> Sign<N> for M {
    fn sign(message: N, _: &Signer) -> Self {
        message.into()
    }
}

pub trait Verify<I> {
    fn verify(&self, verifier: &Verifier<I>) -> Result<(), Invalid>;
}
