use std::{io::Cursor, sync::Arc};

use rcgen::{CertificateParams, DistinguishedName, KeyPair, SanType, PKCS_ED25519};
use tokio::net::TcpStream;
use tokio_rustls::{
    rustls::{
        server::AllowAnyAuthenticatedClient, Certificate, ClientConfig, PrivateKey, RootCertStore,
        ServerConfig, ServerName,
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
    let key_pair = KeyPair::generate(&PKCS_ED25519).unwrap();
    let private_key = PrivateKey(key_pair.serialize_der());
    let mut cert_params = CertificateParams::new(Vec::new());
    cert_params.distinguished_name = DistinguishedName::new();
    cert_params.alg = &PKCS_ED25519;
    cert_params.key_pair = Some(key_pair);
    cert_params
        .subject_alt_names
        .push(SanType::DnsName(String::from("neat.test")));
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

pub type Connection<S, D> = crate::tcp::GeneralConnection<TlsStream<TcpStream>, S, D>;
pub type ConnectionOut = crate::tcp::ConnectionOut;

#[derive(Clone)]
pub struct Connector(pub TlsConnector);

impl Default for Connector {
    fn default() -> Self {
        Self(TlsConnector::from(Arc::new(client_config())))
    }
}

impl Connector {
    pub async fn upgrade_client<S, D>(
        &self,
        connection: crate::tcp::Connection<S, D>,
    ) -> Connection<S, D> {
        let (connection, stream) = connection.replace_stream(());
        let stream = self
            .0
            .connect(
                ServerName::try_from("neat.test").unwrap(),
                stream.into_inner(),
            )
            .await
            .unwrap();
        connection.replace_stream(Client(stream)).0
    }
}

#[derive(Clone)]
pub struct Acceptor(pub TlsAcceptor);

impl Default for Acceptor {
    fn default() -> Self {
        Self(TlsAcceptor::from(Arc::new(server_config())))
    }
}

impl Acceptor {
    pub async fn upgrade_server<S, D>(
        &self,
        connection: crate::tcp::Connection<S, D>,
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
