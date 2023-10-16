use std::{future, sync::Arc, time::Duration};

use sep2_client::client::{Client, SEPResponse};
use sep2_common::packages::{dcap::DeviceCapability, edev::EndDevice, primitives::Uint32};
use sep2_test_server::TestServer;
use tokio::sync::RwLock;

fn test_setup() -> Client {
    Client::new(
        "https://127.0.0.1:1337",
        "../certs/client_cert.pem",
        "../certs/client_private_key.pem",
        None,
        // Make the client constantly check for the latest poll task to service
        Some(Duration::from_secs(0)),
    )
    .unwrap()
}

pub fn all_eq<T: PartialEq>(iter: &[T]) -> bool {
    let mut iter = iter.iter();
    let first = iter.next().unwrap();
    iter.all(|elem| elem == first)
}

// This test simply runs our server for the duration of this modules tests
#[tokio::test]
async fn run_test_server() {
    tokio::spawn(async move {
        let _ = TestServer::new(
            "127.0.0.1:1337",
            "../certs/server_cert.pem",
            "../certs/server_private_key.pem",
        )
        .unwrap()
        .run(future::pending::<()>())
        .await;
    });
    // Dumb, but good enough for now
    tokio::time::sleep(Duration::from_secs(5)).await;
}

#[tokio::test]
async fn basic_req() {
    let client = test_setup();
    client.get::<DeviceCapability>("/dcap").await.unwrap();
    let out = client.post("/edev", &EndDevice::default()).await.unwrap();
    assert!(matches!(out, SEPResponse::Created(_)));
    let out = client.put("/edev/3", &EndDevice::default()).await.unwrap();
    assert!(matches!(out, SEPResponse::NoContent));
    client.delete("/edev/3").await.unwrap();
}

#[tokio::test]
async fn basic_poll() {
    let client = test_setup();
    let output: Arc<RwLock<Vec<DeviceCapability>>> = Arc::new(RwLock::new(vec![]));
    let inner = output.clone();
    client
        .start_poll("/dcap", Some(Uint32(2)), move |r: DeviceCapability| {
            let out = inner.clone();
            async move {
                out.write().await.push(r);
            }
        })
        .await;
    let inner = output.clone();
    client
        .start_poll("/bad_dcap", Some(Uint32(1)), move |r: DeviceCapability| {
            let out = inner.clone();
            async move {
                out.write().await.push(r);
            }
        })
        .await;
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(output.read().await.len(), 2);
    assert!(all_eq(output.read().await.as_ref()));
}
