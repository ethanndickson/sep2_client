use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use client::{
    client::{Client, SepResponse},
    pubsub::{ClientNotifServer, NotifHandler},
};
use common::{
    deserialize,
    packages::{
        primitives::Uint32,
        pubsub::Notification,
        xsd::{DeviceCapability, Reading},
    },
};
use tokio::sync::{mpsc, RwLock};
use typemap_rev::{TypeMap, TypeMapKey};

struct ReadingResource;
impl TypeMapKey for ReadingResource {
    type Value = Reading;
}

#[derive(Default)]
struct Handler {
    // Store any form of state in your handler, and have it accessible in your notification handler
    state: RwLock<TypeMap>,
}

impl Handler {
    async fn add_reading(&self, resource: &str) -> SepResponse {
        let res = deserialize::<Notification<Reading>>(resource);
        match res {
            Ok(r) => {
                self.state
                    .write()
                    .await
                    .insert::<ReadingResource>(r.resource.unwrap());
                SepResponse::Created("/reading".to_owned())
            }
            // "Undesired subscription ... SHALL return an HTTP 400 error"
            Err(_) => SepResponse::BadRequest(None),
        }
    }
}

#[async_trait]
impl NotifHandler for Handler {
    async fn notif_handler(&self, path: &str, resource: &str) -> SepResponse {
        match path {
            "/reading" => self.add_reading(resource).await,
            _ => SepResponse::NotFound,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let notifs = ClientNotifServer::new(
        "127.0.0.1:1338",
        "../certs/client_cert.pem",
        "../certs/client_private_key.pem",
        Handler::default(),
    )?;
    let notif_handle = tokio::task::spawn(notifs.run());
    let client = Client::new(
        "https://127.0.0.1:1337",
        "../certs/client_cert.pem",
        "../certs/client_private_key.pem",
        None,
    )?;
    let dcap = client.get::<DeviceCapability>("/dcap").await?;
    let (tx, mut rx) = mpsc::channel::<String>(100);
    client
        .start_poll(
            dcap.href.unwrap(),
            Some(Uint32(1)),
            move |dcap: DeviceCapability| {
                let tx = tx.clone();
                async move { tx.send(dcap.href.unwrap()).await.unwrap() }
            },
        )
        .await;
    tokio::task::spawn(async move {
        while let Some(url) = rx.recv().await {
            println!("Updated href: {}", url);
        }
    });
    tokio::time::sleep(Duration::from_secs(3)).await;
    client.cancel_polls().await;
    // Join notif task
    notif_handle.await?
}
