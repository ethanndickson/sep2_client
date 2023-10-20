//! TLS Configuration for a IEEE 2030.5 Client & Notification Server
//!
//! Missing from this section is a verification callback that checks the presence of critical & non-critical extensions required by IEEE 2030.5 section 6.11
//! The `rust-openssl` crate we use for openssl bindings currently does not expose an interface necessary to check these extensions: https://github.com/sfackler/rust-openssl/issues/373
//! In the meantime, we use `x509_parser` to parse and verify
//!

use std::time::Duration;

use anyhow::{bail, Result};
use hyper::client::HttpConnector;
use hyper::{Body, Client};
use hyper_openssl::HttpsConnector;
use openssl::ssl::{SslConnector, SslConnectorBuilder, SslFiletype, SslMethod, SslVerifyMode};

#[cfg(feature = "pubsub")]
use openssl::ssl::{SslAcceptor, SslAcceptorBuilder};
use x509_parser::prelude::ParsedExtension;

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
    let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls_server())?;
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

fn check_cert(cert_path: &str) -> Result<()> {
    let contents = std::fs::read(cert_path)?;
    let (_rem, cert) = x509_parser::pem::parse_x509_pem(&contents)?;
    let cert = cert.parse_x509()?;
    let exts = cert.extensions();
    for ext in exts {
        let critical = ext.critical;
        match ext.parsed_extension() {
            // "certificates containing policy mappings MUST be rejected"
            ParsedExtension::PolicyMappings(_) => {
                bail!("Certificates cannot contain policy mappings.")
            }
            // "Name-constraints are not supported and certificates containing name-constraints MUST be rejected."
            ParsedExtension::NameConstraints(_) => {
                bail!("Certificates cannot contain name constraints.")
            }
            // TODO: What do we need to examine inside the rest of these extensions? What can we reasonably check?
            ParsedExtension::KeyUsage(_)
            | ParsedExtension::BasicConstraints(_)
            | ParsedExtension::CertificatePolicies(_)
            | ParsedExtension::SubjectAlternativeName(_) => {
                if !critical {
                    bail!(
                        "KeyUsage, BasicConstraints & CertificatePolicies extensions must be critical."
                    )
                }
            }
            ParsedExtension::AuthorityKeyIdentifier(_)
            | ParsedExtension::SubjectKeyIdentifier(_) => {
                if critical {
                    bail!(
                        "AuthorityKeyIdentifier & SubjectKeyIdentifier extensions cannot be critical."
                    )
                }
            }
            // All other extensions are ignored
            _ => (),
        }
    }
    Ok(())
}
