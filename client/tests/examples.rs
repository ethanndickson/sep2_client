use client::client::Client;
use client::client::SepResponse::Created;
use common::{
    deserialize,
    examples::{ED_16_01_08, REG_16_01_10},
    packages::{
        primitives::{Int64, Uint32, Uint40},
        xsd::{EndDevice, EndDeviceList, Registration},
    },
};

fn test_setup() -> (EndDevice, Registration, Client) {
    // SETUP: Supplied to client (out of band)
    let mut edr = EndDevice::default();
    edr.changed_time = Int64(1379905200);
    edr.s_fdi = Uint40(987654321005);
    let mut reg = Registration::default();
    reg.date_time_registered = Int64(1364774400);
    reg.p_in = Uint32(123455);
    // Create client
    let client = Client::new(
        "127.0.0.1:1337",
        "../certs/client_cert.pem",
        "../certs/client_private_key.pem",
    )
    .unwrap();
    (edr, reg, client)
}

/// IEEE 2030.5-2018 - Table C.1
#[tokio::test]
async fn registration_remote() {
    let (own_edr, own_reg, client) = test_setup();
    // Verify End Device on server
    let edr: EndDevice = client.get("/edev/3").await.unwrap();
    let expected_edr: EndDevice = deserialize(ED_16_01_08).unwrap();
    assert_eq!(expected_edr, edr);
    assert_eq!(own_edr.s_fdi, edr.s_fdi);
    // Verify pin matches
    let reg: Registration = client.get("/edev/3/reg").await.unwrap();
    let expected_reg: Registration = deserialize(REG_16_01_10).unwrap();
    assert_eq!(expected_reg, reg);
    assert_eq!(own_reg.p_in, reg.p_in);
}

#[tokio::test]
async fn registration_local() {
    let (mut own_edr, _, client) = test_setup();
    // Test-specific SFDI
    own_edr.s_fdi = Uint40(789654321005);
    // Verify our SFDI isn't in the server's list
    let edrl: EndDeviceList = client.get("/edev").await.unwrap();
    for each_ed in edrl.end_device {
        assert_ne!(each_ed.s_fdi, own_edr.s_fdi);
    }
    let res = client.post("/edev", &own_edr).await.unwrap();
    if let Created(loc) = res {
        assert_eq!(loc, "/edev/4");
    } else {
        panic!("Expected 201 Created from server, not 204 No Content");
    }
}
