use std::{io::Cursor, sync::Arc};

use rcgen::{CertificateParams, DistinguishedName, KeyPair, PKCS_ECDSA_P256_SHA256};
use tokio::net::TcpStream;
use tokio_rustls::{
    rustls::{
        server::AllowAnyAuthenticatedClient, Certificate, ClientConfig, PrivateKey, RootCertStore,
        ServerConfig, ServerName::IpAddress,
    },
    TlsAcceptor, TlsConnector,
    TlsStream::{self, Client, Server},
};

const CERT: &str = include_str!("ca-cert.pem");
const KEY: &str = include_str!("ca-key.pem");

fn root_store() -> RootCertStore {
    let mut store = RootCertStore::empty();
    store.add_parsable_certificates(&rustls_pemfile::certs(&mut Cursor::new(CERT)).unwrap());
    store
}

fn generate() -> (PrivateKey, Certificate) {
    let key_pair = KeyPair::generate(&PKCS_ECDSA_P256_SHA256).unwrap();
    let private_key = PrivateKey(key_pair.serialize_der());
    let mut cert_params = CertificateParams::new(Vec::new());
    cert_params.distinguished_name = DistinguishedName::new();
    cert_params.alg = &PKCS_ECDSA_P256_SHA256;
    cert_params.key_pair = Some(key_pair);
    let cert = rcgen::Certificate::from_params(cert_params).unwrap();
    let root_key_pair = KeyPair::from_pem(KEY).unwrap();
    let root_cert = rcgen::Certificate::from_params(
        rcgen::CertificateParams::from_ca_cert_pem(CERT, root_key_pair).unwrap(),
    )
    .unwrap();
    (
        private_key,
        Certificate(cert.serialize_der_with_signer(&root_cert).unwrap()),
    )
}

fn client_config() -> ClientConfig {
    let (private_key, cert) = generate();
    ClientConfig::builder()
        .with_safe_defaults()
        .with_root_certificates(root_store())
        .with_single_cert(vec![cert], private_key)
        .unwrap()
}

fn server_config() -> ServerConfig {
    let (private_key, cert) = generate();
    ServerConfig::builder()
        .with_safe_defaults()
        .with_client_cert_verifier(AllowAnyAuthenticatedClient::new(root_store()).boxed())
        .with_single_cert(vec![cert], private_key)
        .unwrap()
}

pub type Connection<S, D> = larlis_tcp::GeneralConnection<S, D, TlsStream<TcpStream>>;
pub type ConnectionOut = larlis_tcp::ConnectionOut;

pub async fn upgrade_client<S, D>(connection: larlis_tcp::Connection<S, D>) -> Connection<S, D> {
    let (connection, stream) = connection.replace_stream(());
    let stream = TlsConnector::from(Arc::new(client_config()))
        .connect(IpAddress(connection.remote_addr.ip()), stream.into_inner())
        .await
        .unwrap();
    connection.replace_stream(Client(stream)).0
}

pub struct Accept(pub TlsAcceptor);

impl Default for Accept {
    fn default() -> Self {
        Self(TlsAcceptor::from(Arc::new(server_config())))
    }
}

impl Accept {
    pub async fn upgrade_server<S, D>(
        &self,
        connection: larlis_tcp::Connection<S, D>,
    ) -> Connection<S, D> {
        let (connection, stream) = connection.replace_stream(());
        let stream = self.0.accept(stream.into_inner()).await.unwrap();
        connection.replace_stream(Server(stream)).0
    }
}

#[cfg(test)]
mod tests {
    use rcgen::{CertificateParams, KeyPair};

    use super::*;

    #[test]
    fn valid_cert() {
        let key_pair = KeyPair::from_pem(KEY).unwrap();
        let params = CertificateParams::from_ca_cert_pem(CERT, key_pair).unwrap();
        assert!(params.key_pair.is_some());
    }
}
