use std::time::Duration;

use anyhow::Result;
use hyper::client::HttpConnector;
use hyper::{Body, Client};
use hyper_openssl::HttpsConnector;
use openssl::ssl::{
    SslAcceptor, SslAcceptorBuilder, SslConnector, SslConnectorBuilder, SslFiletype, SslMethod,
};
pub(crate) type Connector = HttpsConnector<HttpConnector>;
pub(crate) type HTTPSClient = Client<Connector, Body>;
pub(crate) type TlsClientConfig = SslConnectorBuilder;

pub(crate) fn create_client_tls_cfg(cert_path: &str, pk_path: &str) -> Result<TlsClientConfig> {
    let mut builder = SslConnector::builder(SslMethod::tls_client())?;
    log::debug!("Setting CipherSuite");
    builder.set_cipher_list("ECDHE-ECDSA-AES128-CCM8")?;
    log::debug!("Loading Certificate File");
    builder.set_certificate_file(cert_path, SslFiletype::PEM)?;
    log::debug!("Loading Private Key File");
    builder.set_private_key_file(pk_path, SslFiletype::PEM)?;
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

pub type TlsServerConfig = SslAcceptorBuilder;

pub fn create_server_tls_config(cert_path: &str, pk_path: &str) -> Result<TlsServerConfig> {
    let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls_server()).unwrap();
    log::debug!("Setting CipherSuite");
    builder.set_cipher_list("ECDHE-ECDSA-AES128-CCM8")?;
    log::debug!("Loading Certificate File");
    builder.set_certificate_file(cert_path, SslFiletype::PEM)?;
    log::debug!("Loading Private Key File");
    builder.set_private_key_file(pk_path, SslFiletype::PEM)?;
    Ok(builder)
}
