use log::debug;
use openssl::ssl::{SslAcceptor, SslAcceptorBuilder, SslFiletype, SslMethod};
use std::error::Error;

pub(crate) type TlsConfig = SslAcceptorBuilder;

pub(crate) fn create_tls_config(
    cert_path: &str,
    pk_path: &str,
) -> Result<TlsConfig, Box<dyn Error + Send + Sync>> {
    let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls_server()).unwrap();
    debug!("Setting CipherSuite");
    builder.set_cipher_list("ECDHE-ECDSA-AES128-CCM8")?;
    debug!("Loading Certificate File");
    builder.set_certificate_file(cert_path, SslFiletype::PEM)?;
    debug!("Loading Private Key File");
    builder.set_private_key_file(pk_path, SslFiletype::PEM)?;
    Ok(builder)
}
