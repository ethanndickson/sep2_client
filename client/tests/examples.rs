use client::client::Client;
use client::client::SepResponse::Created;
use common::{
    deserialize,
    examples::{ED_16_01_08, REG_16_01_10},
    packages::{
        primitives::{Int64, Uint32, Uint40},
        xsd::{
            DeviceCapability, EndDevice, EndDeviceList, FunctionSetAssignmentsList, Registration,
        },
    },
};

// Possible Happy Path Example Tests from the IEEE 2030.5 Specification

// Supplied to client as starting resources (out of band)
fn test_setup() -> (EndDevice, Registration, Client) {
    let mut edr = EndDevice::default();
    edr.changed_time = Int64(1379905200);
    edr.sfdi = Uint40(987654321005);
    let mut reg = Registration::default();
    reg.date_time_registered = Int64(1364774400);
    reg.p_in = Uint32(123455);
    // Create client
    let client = Client::new(
        "https://127.0.0.1:1337",
        "../certs/client_cert.pem",
        "../certs/client_private_key.pem",
        None,
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
    assert_eq!(own_edr.sfdi, edr.sfdi);
    // Verify pin matches
    let reg: Registration = client.get("/edev/3/reg").await.unwrap();
    let expected_reg: Registration = deserialize(REG_16_01_10).unwrap();
    assert_eq!(expected_reg, reg);
    assert_eq!(own_reg.p_in, reg.p_in);
}

/// IEEE 2030.5-2018 - Table C.3
#[tokio::test]
async fn registration_local() {
    let (mut own_edr, _, client) = test_setup();
    // Test-specific SFDI
    own_edr.sfdi = Uint40(789654321005);
    // Verify our SFDI isn't in the server's list
    let edrl: EndDeviceList = client.get("/edev").await.unwrap();
    for each_ed in edrl.end_device {
        assert_ne!(each_ed.sfdi, own_edr.sfdi);
    }
    let res = client.post("/edev", &own_edr).await.unwrap();
    // Header should return location of newly posted resource
    if let Created(loc) = res {
        assert_eq!(loc, "/edev/4");
    } else {
        panic!("Expected 201 Created from server, not 204 No Content");
    }
}

/// IEEE 2030.5-2018 - Table C.4
#[tokio::test]
async fn function_set_assignment() {
    let (_, _, client) = test_setup();
    // Get own EndDevice resource
    let edr: EndDevice = client.get("/edev/4").await.unwrap();
    // Get link to Function Set Assignment List
    let fsal = edr.function_set_assignments_list_link.unwrap();
    // Query FSA List
    let fsal: FunctionSetAssignmentsList = client
        .get(&format!("{}?l={}", fsal.href, fsal.all.unwrap()))
        .await
        .unwrap();
    // Search list for Demand Response Program List Link
    let fsa = fsal
        .function_set_assignments
        .iter()
        .find(|e| e.demand_response_program_list_link.is_some())
        .unwrap();
    // Get Demand Response Program List Link
    fsa.demand_response_program_list_link.as_ref().unwrap();
}

/// IEEE 2030.5-2018 - table C.5
#[tokio::test]
async fn no_function_set_assignment() {
    let (_, _, client) = test_setup();
    // Get EDR
    let edr: EndDevice = client.get("/edev/5").await.unwrap();
    // Discover there is no FSA
    assert!(edr.function_set_assignments_list_link.is_none());
    // Fallback and get DeviceCapabilities (URI determined out of band or during DNS-SD)
    let dc: DeviceCapability = client.get("/dcap").await.unwrap();
    // Get DRPLL
    dc.demand_response_program_list_link.unwrap();
}
