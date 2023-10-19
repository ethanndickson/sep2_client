//! TLS Configuration for a IEEE 2030.5 Client & Notification Server
//!
//! Missing from this section is a verification callback that checks the presence of critical & non-critical extensions required by IEEE 2030.5 sections 8.11.6, 8.11.8, and 8.11.10
//! The `rust-openssl` crate we use for openssl bindings currently does not expose the extensions necessary to check these extensions: https://github.com/sfackler/rust-openssl/issues/373
//! TODO: In the meantime, we use `x509_parser` to verify the presence of extensions in the given certificates
//!

use std::time::Duration;

use anyhow::Result;
use hyper::client::HttpConnector;
use hyper::{Body, Client};
use hyper_openssl::HttpsConnector;
use openssl::ssl::{SslConnector, SslConnectorBuilder, SslFiletype, SslMethod, SslVerifyMode};

#[cfg(feature = "pubsub")]
use openssl::ssl::{SslAcceptor, SslAcceptorBuilder};

pub(crate) type Connector = HttpsConnector<HttpConnector>;
pub(crate) type HTTPSClient = Client<Connector, Body>;
pub(crate) type TlsClientConfig = SslConnectorBuilder;

pub(crate) fn create_client_tls_cfg(
    cert_path: &str,
    pk_path: &str,
    rootca_path: &str,
) -> Result<TlsClientConfig> {
    let mut builder = SslConnector::builder(SslMethod::tls_client())?;
    log::debug!("Setting CipherSuite");
    builder.set_cipher_list("ECDHE-ECDSA-AES128-CCM8")?;
    log::debug!("Loading Certificate File");
    builder.set_certificate_file(cert_path, SslFiletype::PEM)?;
    log::debug!("Loading Private Key File");
    builder.set_private_key_file(pk_path, SslFiletype::PEM)?;
    log::debug!("Loading Certificate Authority File");
    builder.set_ca_file(rootca_path)?;
    log::debug!("Setting verification mode");
    builder.set_verify(SslVerifyMode::PEER);
    Ok(builder)
}

pub(crate) fn create_client(
    tls_config: TlsClientConfig,
    tcp_keepalive: Option<Duration>,
) -> Client<Connector, Body> {
    let mut http = HttpConnector::new();
    http.enforce_http(false);
    http.set_keepalive(tcp_keepalive);
    let https = HttpsConnector::with_connector(http, tls_config).unwrap();
    Client::builder().build::<Connector, hyper::Body>(https)
}

#[cfg(feature = "pubsub")]
pub(crate) type TlsServerConfig = SslAcceptorBuilder;

#[cfg(feature = "pubsub")]
pub(crate) fn create_server_tls_config(
    cert_path: &str,
    pk_path: &str,
    rootca_path: &str,
) -> Result<TlsServerConfig> {
    // rust-openssl forces us to create this default config that we immediately overwrite
    // If they gave us a way to cosntruct Acceptors and Connectors from Contexts,
    // we wouldn't need to double up on configs here
    let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls_server()).unwrap();
    log::debug!("Setting CipherSuite");
    builder.set_cipher_list("ECDHE-ECDSA-AES128-CCM8")?;
    log::debug!("Loading Certificate File");
    builder.set_certificate_file(cert_path, SslFiletype::PEM)?;
    log::debug!("Loading Private Key File");
    builder.set_private_key_file(pk_path, SslFiletype::PEM)?;
    log::debug!("Loading Certificate Authority File");
    builder.set_ca_file(rootca_path)?;
    log::debug!("Setting verification mode");
    builder.set_verify(SslVerifyMode::FAIL_IF_NO_PEER_CERT | SslVerifyMode::PEER);
    Ok(builder)
}
