use sep2_client::{
    client::{Client, SEPResponse},
    pubsub::ClientNotifServer,
};
use sep2_common::packages::{dcap::DeviceCapability, edev::EndDevice, pubsub::Notification};
use std::{future, time::Duration};

fn test_setup() -> Client {
    Client::new(
        // Point to Notif Server
        "https://127.0.0.1:1338",
        "../certs/server_cert.pem",
        "../certs/server_private_key.pem",
        None,
        None,
    )
    .unwrap()
}

#[tokio::test]
async fn run_notif_server() {
    let client = ClientNotifServer::new(
        "127.0.0.1:1338",
        "../certs/client_cert.pem",
        "../certs/client_private_key.pem",
    )
    .unwrap()
    .add("/dcap", |_: Notification<DeviceCapability>| async move {
        SEPResponse::Created(None)
    })
    .add("/edev", |_: Notification<EndDevice>| async move {
        SEPResponse::Created(None)
    });
    tokio::spawn(client.run(future::pending::<()>()));
    tokio::time::sleep(Duration::from_secs(2)).await;
}

#[tokio::test]
async fn basic_post() {
    let notif: Notification<DeviceCapability> = Default::default();
    let client = test_setup();
    // Valid
    assert!(matches!(
        client.post("/dcap", &notif).await.unwrap(),
        SEPResponse::Created(None)
    ));
    assert!(matches!(
        client.post("/edev", &notif).await.unwrap(),
        SEPResponse::Created(None)
    ));
    // Bad method
    assert!(client.put("/edev", &notif).await.is_err());
    assert!(client.delete("/edev").await.is_err());
    assert!(client.get::<EndDevice>("/edev").await.is_err());
    // Bad route
    assert!(client.post("/bad_route", &notif).await.is_err());
    assert!(client.post("", &notif).await.is_err());
    assert!(client.post("/", &notif).await.is_err());
}
