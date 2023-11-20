#![cfg(feature = "messaging")]
use sep2_client::{
    client::{Client, SEPResponse},
    pubsub::{ClientNotifServer, RouteCallback},
};
use sep2_common::packages::{dcap::DeviceCapability, edev::EndDevice, pubsub::Notification};
use std::{convert::Infallible, future, time::Duration};

fn test_setup() -> Client {
    Client::new_https(
        // Point to Notif Server
        "https://127.0.0.1:1338",
        "../certs/client_cert.pem",
        "../certs/client_private_key.pem",
        "../certs/rootCA.pem",
        None,
        None,
    )
    .unwrap()
}

#[derive(Clone)]
struct DCAPHandler;

impl RouteCallback<DeviceCapability> for DCAPHandler {
    fn callback(
        &self,
        _: Notification<DeviceCapability>,
    ) -> std::pin::Pin<Box<dyn future::Future<Output = SEPResponse> + Send + 'static>> {
        Box::pin(async move { SEPResponse::Created(None) })
    }
}

#[tokio::test]
async fn run_notif_server() {
    let client = ClientNotifServer::new("127.0.0.1:1338")
        .unwrap()
        .with_https(
            "../certs/server_cert.pem",
            "../certs/server_private_key.pem",
            "../certs/rootCA.pem",
        )
        .unwrap()
        .add("/dcap", DCAPHandler)
        .add("/edev", |_: Notification<EndDevice>| async move {
            SEPResponse::Created(None)
        });
    tokio::spawn(client.run(future::pending::<Infallible>()));
    tokio::time::sleep(Duration::from_secs(4)).await;
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
    assert!(matches!(
        client.put("/edev", &notif).await.unwrap(),
        SEPResponse::MethodNotAllowed(_),
    ));
    assert!(matches!(
        client.delete("/edev").await.unwrap(),
        SEPResponse::MethodNotAllowed(_),
    ));
    assert!(client.get::<EndDevice>("/edev").await.is_err());
    // Bad route
    assert!(matches!(
        client.post("/bad_route", &notif).await.unwrap(),
        SEPResponse::NotFound,
    ));
    assert!(matches!(
        client.post("", &notif).await.unwrap(),
        SEPResponse::NotFound,
    ));
    assert!(matches!(
        client.post("/", &notif).await.unwrap(),
        SEPResponse::NotFound,
    ));
}
