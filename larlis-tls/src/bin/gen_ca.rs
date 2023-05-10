use p256::ecdsa::SigningKey;
use p256::pkcs8::EncodePrivateKey;
use rand::rngs::OsRng;

use rcgen::{date_time_ymd, Certificate, CertificateParams, DistinguishedName};
use std::convert::TryFrom;
use std::fs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut params: CertificateParams = Default::default();
    params.not_before = date_time_ymd(2023, 5, 10);
    params.not_after = date_time_ymd(4096, 1, 1);
    params.distinguished_name = DistinguishedName::new();

    params.alg = &rcgen::PKCS_ECDSA_P256_SHA256;

    let mut rng = OsRng;
    let private_key = SigningKey::random(&mut rng);
    let private_key_der = private_key.to_pkcs8_der()?;
    let key_pair = rcgen::KeyPair::try_from(private_key_der.as_bytes()).unwrap();
    params.key_pair = Some(key_pair);

    let cert = Certificate::from_params(params)?;
    fs::write("ca-cert.pem", cert.serialize_pem()?)?;
    fs::write("ca-key.pem", cert.serialize_private_key_pem())?;
    Ok(())
}
