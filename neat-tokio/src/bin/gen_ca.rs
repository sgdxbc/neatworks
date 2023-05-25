use rcgen::{date_time_ymd, Certificate, CertificateParams, DistinguishedName};

use std::fs;
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut params: CertificateParams = Default::default();
    params.not_before = date_time_ymd(1970, 1, 1);
    params.not_after = date_time_ymd(4096, 1, 1);
    params.distinguished_name = DistinguishedName::new();
    params.alg = &rcgen::PKCS_ED25519;

    let cert = Certificate::from_params(params)?;
    let out_dir = Path::new(file!()).parent().unwrap().parent().unwrap();
    fs::write(out_dir.join("ca-cert.pem"), cert.serialize_pem()?)?;
    fs::write(out_dir.join("ca-key.pem"), cert.serialize_private_key_pem())?;
    Ok(())
}
