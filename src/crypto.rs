use std::ops::{Deref, DerefMut};

use borsh::{BorshDeserialize, BorshSerialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Untrusted {
    signature: Signature,
    inner: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Signature([u8; 64]);

impl BorshSerialize for Untrusted {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        self.signature.serialize(writer)?;
        writer.write_all(&self.inner)
    }
}

impl BorshDeserialize for Untrusted {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let signature = Signature::deserialize_reader(reader)?;
        let mut inner = Vec::new();
        reader.read_to_end(&mut inner)?;
        Ok(Self { signature, inner })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified<M> {
    pub signature: Signature,
    pub inner: M,
    serialized_inner: Vec<u8>,
}

impl<M> Deref for Verified<M> {
    type Target = M;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<M> DerefMut for Verified<M> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl<M> From<Verified<M>> for Untrusted {
    fn from(value: Verified<M>) -> Self {
        Self {
            inner: value.serialized_inner,
            signature: value.signature,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Signer(secp256k1::SecretKey);

impl Signer {
    pub fn serialize_sign<M>(&self, message: M) -> crate::Result<Verified<M>>
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
        Ok(Verified {
            signature,
            inner: message,
            serialized_inner,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verifier(secp256k1::PublicKey);

impl Verifier {
    pub fn verify_deserialize<M>(self, untrusted: Untrusted) -> crate::Result<Verified<M>>
    where
        M: BorshDeserialize,
    {
        let hash = secp256k1::Message::from_hashed_data::<secp256k1::hashes::sha256::Hash>(
            &untrusted.inner,
        );
        thread_local! {
            static SECP: secp256k1::Secp256k1<secp256k1::VerifyOnly> =
                secp256k1::Secp256k1::verification_only();
        }
        let Signature(signature) = &untrusted.signature;
        SECP.with(|secp| {
            secp.verify_ecdsa(
                &hash,
                &secp256k1::ecdsa::Signature::from_compact(signature)?,
                &self.0,
            )
        })?;
        Ok(Verified {
            signature: untrusted.signature,
            inner: borsh::from_slice(&untrusted.inner)?,
            serialized_inner: untrusted.inner,
        })
    }
}
