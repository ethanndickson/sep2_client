//! TLS Configuration for a IEEE 2030.5 Client & Notification Server
//!
//! Missing from this section is a verification callback that checks the presence of critical & non-critical extensions required by IEEE 2030.5 section 6.11
//! The `rust-openssl` crate we use for openssl bindings currently does not expose an interface necessary to check these extensions: <https://github.com/sfackler/rust-openssl/issues/373>
//! In the meantime, we use `x509_parser` to parse and verify that the required extensions are present, for both self-signed Client Certificates and device certificates, as per the specification.
//!
//!

use std::path::Path;
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
    cert_path: impl AsRef<Path>,
    pk_path: impl AsRef<Path>,
    rootca_path: impl AsRef<Path>,
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
    cert_path: impl AsRef<Path>,
    pk_path: impl AsRef<Path>,
    rootca_path: impl AsRef<Path>,
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

/// Verify that the PEM encoded certificate at the given path meets IEEE 2030.5 "Device Certificate" requirements.
/// Newly purchased or acquired certificates in an IEEE 2030.5 certificate chain will satisfy these requirements.
///
/// Currently this function isn't called when instantiating a [`Client`]` nor a [`ClientNotifServer`], but that may change in the future.
///
///
/// A valid 'device certificate' *must* be used by a [`ClientNotifServer`], i.e. NOT a self signed certificate
/// "The use of TLS (IETF RFC 5246) requires that all hosts implementing server functionality SHALL use a
/// device certificate whereby the server presents its device certificate as part of the TLS handshake"
///
/// See section 6.11.8.3.3 for more.
///
/// [`Client`]: crate::client::Client
/// [`ClientNotifServer`]: crate::pubsub::ClientNotifServer
pub fn check_device_cert(cert_path: impl AsRef<Path>) -> Result<()> {
    let contents = std::fs::read(cert_path)?;
    let (_rem, cert) = x509_parser::pem::parse_x509_pem(&contents)?;
    let cert = cert.parse_x509()?;
    // TODO: Check Issued by & Subject name
    let exts = cert.extensions();
    let mut key_usage = false;
    let mut certificate_policies = false;
    let mut san = false;
    let mut aki = false;
    for ext in exts {
        let critical = ext.critical;
        match ext.parsed_extension() {
            // "certificates containing policy mappings MUST be rejected"
            ParsedExtension::PolicyMappings(_) => {
                bail!("Device Certificates cannot contain policy mappings.")
            }
            // "Name-constraints are not supported and certificates containing name-constraints MUST be rejected."
            ParsedExtension::NameConstraints(_) => {
                bail!("Device Certificates cannot contain name constraints.")
            }
            // TODO: What do we need to examine inside the rest of these extensions? What can we reasonably check?
            ParsedExtension::CertificatePolicies(_) => {
                if critical {
                    certificate_policies = true;
                } else {
                    bail!("CertificatePolicies extension must be critical.")
                }
            }
            ParsedExtension::SubjectAlternativeName(_) => {
                if critical {
                    san = true;
                } else {
                    bail!("SubjectAlternativeName extension must be critical")
                }
            }
            ParsedExtension::KeyUsage(_) => {
                if critical {
                    key_usage = true;
                } else {
                    bail!("KeyUsage extension must be critical.")
                }
            }
            ParsedExtension::AuthorityKeyIdentifier(_) => {
                if critical {
                    bail!("AuthorityKeyIdentifier extension cannot be critical.")
                } else {
                    aki = true;
                }
            }
            // All other extensions constitute an invalid certificate
            // TODO: This might need to be relaxed to allow for modifications
            _ => bail!("Unexpected extension or unparsed extension encountered."),
        }
    }
    if !key_usage {
        bail!("KeyUsage extension not present")
    }
    if !certificate_policies {
        bail!("CertificatePolicies extension not present")
    }
    if !san {
        bail!("SubjectAlternativeName extension not present")
    }
    if !aki {
        bail!("AuthorityKeyIdentifier extension not present")
    }
    Ok(())
}

/// Verify that the PEM encoded certificate at the given path meets IEEE 2030.5 "Self Signed Client Certificate" requirements.
///
/// See Section 6.11.8.4.3 for more
pub fn check_self_signed_client_cert(cert_path: impl AsRef<Path>) -> Result<()> {
    let contents = std::fs::read(cert_path)?;
    let (_rem, cert) = x509_parser::pem::parse_x509_pem(&contents)?;
    let cert = cert.parse_x509()?;
    let exts = cert.extensions();
    let mut key_usage = false;
    let mut certificate_policies = false;
    // TODO: Check Issued by, Subject Name, Issuer Name, Validity, and Subject Public Key and Signature
    for ext in exts {
        let critical = ext.critical;
        match ext.parsed_extension() {
            // "certificates containing policy mappings MUST be rejected"
            ParsedExtension::PolicyMappings(_) => {
                bail!("Device Certificates cannot contain policy mappings.")
            }
            // "Name-constraints are not supported and certificates containing name-constraints MUST be rejected."
            ParsedExtension::NameConstraints(_) => {
                bail!("Device Certificates cannot contain name constraints.")
            }
            ParsedExtension::CertificatePolicies(_) => {
                if critical {
                    certificate_policies = true;
                } else {
                    bail!("CertificatePolicies extension must be critical.")
                }
            }
            ParsedExtension::KeyUsage(_) => {
                if critical {
                    key_usage = true;
                } else {
                    bail!("KeyUsage extension must be critical.")
                }
            }
            // All other extensions constitute an invalid certificate
            // TODO: This might need to be relaxed to allow for modifications
            _ => bail!("Unexpected extension or unparsed extension encountered."),
        }
    }
    if !key_usage {
        bail!("KeyUsage extension not present.")
    }
    if !certificate_policies {
        bail!("CertificatePolicies extension not present.")
    }
    Ok(())
}

// TODO: Should we do checks on the supplied root ca?
