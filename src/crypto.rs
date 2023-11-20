//! Digital signature solution.
//!
//! In this design, signing/verifying is performed against *serialized* messages
//! (`Packet`s) to avoid double traversal for serializing and hashing.
//! `borsh` is used for deterministic serialization, so every type that will
//! get signed or hashed should derive `BorshSerialize`.

use std::ops::Deref;

use borsh::{BorshDeserialize, BorshSerialize};

pub type Digest = [u8; 32];

pub fn digest(data: &impl BorshSerialize) -> crate::Result<Digest> {
    Ok(
        *secp256k1::Message::from_hashed_data::<secp256k1::hashes::sha256::Hash>(&borsh::to_vec(
            data,
        )?)
        .as_ref(),
    )
}

/// On-wire format of signed messages. Happen to be type-erasured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packet {
    signature: Signature,
    inner: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Signature([u8; 64]);

impl BorshSerialize for Packet {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        self.signature.serialize(writer)?;
        writer.write_all(&self.inner)
    }
}

impl BorshDeserialize for Packet {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let signature = Signature::deserialize_reader(reader)?;
        let mut inner = Vec::new();
        reader.read_to_end(&mut inner)?;
        Ok(Self { signature, inner })
    }
}

/// In-memory representation of `M`, equiped with a signature that may or may 
/// not be verified.
/// 
/// If `inner` is mutated, the `inner_bytes` and `signature` may get out of sync
/// with it, so keep `Message<M>` read only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message<M> {
    pub signature: Signature,
    pub inner: M,
    pub verified: bool,
    inner_bytes: Vec<u8>,
}

impl<M> Deref for Message<M> {
    type Target = M;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<M> From<Message<M>> for Packet {
    fn from(value: Message<M>) -> Self {
        Self {
            inner: value.inner_bytes,
            signature: value.signature,
        }
    }
}

impl Packet {
    pub fn deserialize<M>(self) -> crate::Result<Message<M>>
    where
        M: BorshDeserialize,
    {
        Ok(Message {
            signature: self.signature,
            inner: borsh::from_slice(&self.inner)?,
            inner_bytes: self.inner,
            verified: false,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Signer(secp256k1::SecretKey);

impl Signer {
    pub fn serialize_sign<M>(&self, message: M) -> crate::Result<Message<M>>
    where
        M: BorshSerialize,
    {
        let serialized_inner = borsh::to_vec(&message)?;
        let hash = secp256k1::Message::from_hashed_data::<secp256k1::hashes::sha256::Hash>(
            &serialized_inner,
        );
        thread_local! {
            static SECP: secp256k1::Secp256k1<secp256k1::SignOnly> = secp256k1::Secp256k1::signing_only();
        }
        let signature = Signature(
            SECP.with(|secp| secp.sign_ecdsa(&hash, &self.0))
                .serialize_compact(),
        );
        Ok(Message {
            signature,
            inner: message,
            verified: true,
            inner_bytes: serialized_inner,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verifier(secp256k1::PublicKey);

impl Verifier {
    pub fn verify<M>(&self, message: &mut Message<M>) -> crate::Result<()> {
        if message.verified {
            return Ok(());
        }
        let hash = secp256k1::Message::from_hashed_data::<secp256k1::hashes::sha256::Hash>(
            &message.inner_bytes,
        );
        thread_local! {
            static SECP: secp256k1::Secp256k1<secp256k1::VerifyOnly> =
                secp256k1::Secp256k1::verification_only();
        }
        let Signature(signature) = &message.signature;
        SECP.with(|secp| {
            secp.verify_ecdsa(
                &hash,
                &secp256k1::ecdsa::Signature::from_compact(signature)?,
                &self.0,
            )
        })?;
        message.verified = true;
        Ok(())
    }
}
