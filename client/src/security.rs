use anyhow::Result;
use openssl::sha::Sha256;
use sep2_common::packages::{primitives::HexBinary160, types::SFDIType};

/// Generate a LFDI hash value from a certificate file
pub fn lfdi_gen(cert_path: &str) -> Result<HexBinary160> {
    let cert = std::fs::read(cert_path)?;
    let mut hasher = Sha256::new();
    hasher.update(&cert);
    let mut out: [u8; 20] = Default::default();
    out.copy_from_slice(&hasher.finish()[0..20]);
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

/// Given the path to a client certificate, generate a LFDI Hash value, and the corresponding SFDI value.
pub fn security_init(cert_path: &str) -> Result<(HexBinary160, SFDIType)> {
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
