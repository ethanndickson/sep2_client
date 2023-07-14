use hyper::client::HttpConnector;
use hyper::{Body, Client};
use hyper_openssl::HttpsConnector;
use log::debug;
use openssl::ssl::{SslConnector, SslConnectorBuilder, SslFiletype, SslMethod};
use std::io::Result;

pub(crate) type Connector = HttpsConnector<HttpConnector>;
pub(crate) type TlsConfig = SslConnectorBuilder;
pub(crate) type HTTPSClient = Client<Connector, Body>;

pub(crate) fn create_tls_config(cert_path: &str, pk_path: &str) -> Result<TlsConfig> {
    let mut builder = SslConnector::builder(SslMethod::tls_client())?;
    debug!("Setting CipherSuite");
    builder.set_cipher_list("ECDHE-ECDSA-AES128-CCM8")?;
    debug!("Loading Certificate File");
    builder.set_certificate_file(cert_path, SslFiletype::PEM)?;
    debug!("Loading Private Key File");
    builder.set_private_key_file(pk_path, SslFiletype::PEM)?;
    Ok(builder)
}

pub(crate) fn create_client(tls_config: TlsConfig) -> Client<Connector, Body> {
    let mut http = HttpConnector::new();
    http.enforce_http(false);
    let https = HttpsConnector::with_connector(http, tls_config).unwrap();
    Client::builder().build::<Connector, hyper::Body>(https)
}
