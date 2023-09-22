use sep2_client::security::{security_init, sfdi_gen};
use sep2_common::packages::primitives::HexBinary160;

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
