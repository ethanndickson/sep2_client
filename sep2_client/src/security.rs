//! Security Function Set

use std::path::Path;

use anyhow::{anyhow, Result};
use sep2_common::packages::{primitives::HexBinary160, types::SFDIType};
use sha2::{Digest, Sha256};
use x509_parser::{parse_x509_certificate, pem::parse_x509_pem};

/// Generate a LFDI hash value from a PEM or DER certificate file
pub fn lfdi_gen(cert_path: impl AsRef<Path>) -> Result<HexBinary160> {
    let cert = std::fs::read(cert_path)?;
    let der = match parse_x509_pem(&cert) {
        Ok((_, pem)) => pem.contents,
        Err(_) => match parse_x509_certificate(&cert) {
            Ok(_) => cert,
            Err(_) => Err(anyhow!("Unknown certificate format, expected DER or PEM"))?,
        },
    };
    let mut hasher = Sha256::new();
    hasher.update(&der);
    let mut out: [u8; 20] = Default::default();
    out.copy_from_slice(&hasher.finalize()[0..20]);
    Ok(HexBinary160(out))
}

/// Generate a SFDI hash value from a LFDI hash value
pub fn sfdi_gen(lfdi: &HexBinary160) -> SFDIType {
    let mut sfdi: u64 = 0;
    for i in 0..5 {
        sfdi = (sfdi << 8) + u64::from(lfdi[i]);
    }
    sfdi >>= 4;
    SFDIType::new(sfdi * 10 + check_digit(sfdi)).unwrap()
}

/// Given the path to a client PEM or DER certificate, generate a LFDI Hash value, and the corresponding SFDI value.
pub fn security_init(cert_path: impl AsRef<Path>) -> Result<(HexBinary160, SFDIType)> {
    let lfdi = lfdi_gen(cert_path)?;
    let sfdi = sfdi_gen(&lfdi);
    Ok((lfdi, sfdi))
}

// Generate a Luhn Algorithm Check Digit
fn check_digit(mut x: u64) -> u64 {
    let mut sum = 0;
    while x != 0 {
        sum += x % 10;
        x /= 10;
    }
    (10 - (sum % 10)) % 10
}

#[test]
fn security_generates() {
    let _ = security_init("../certs/client_cert.pem").unwrap();
}

/// Generating SFDI as per the specification
#[test]
fn example_sfdi_gen() {
    let lfdi = HexBinary160([
        0x3E, 0x4F, 0x45, 0xAB, 0x31, 0xED, 0xFE, 0x5B, 0x67, 0xE3, 0x43, 0xE5, 0xE4, 0x56, 0x2E,
        0x31, 0x98, 0x4E, 0x23, 0xE5,
    ]);
    let sfdi = sfdi_gen(&lfdi);
    assert_eq!(sfdi.get(), 167261211391)
}
