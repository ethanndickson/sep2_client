use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use client::{
    client::{Client, SepResponse},
    event::{EIStatus, EventHandler, EventInstance},
    pubsub::{ClientNotifServer, NotifHandler},
};
use common::{
    deserialize,
    packages::{
        identification::ResponseStatus,
        objects::DERControl,
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
    // Store any form of interior-mutable state, or a channel inlet here and
    // have it accessible when handling notifs or events.
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

// Example defintion for how incoming notifications should be routed.
#[async_trait]
impl NotifHandler for Handler {
    async fn notif_handler(&self, path: &str, resource: &str) -> SepResponse {
        match path {
            "/reading" => self.add_reading(resource).await,
            _ => SepResponse::NotFound,
        }
    }
}

// Example definition of how DER event status updates should be handled.
#[async_trait]
impl EventHandler<DERControl> for Handler {
    async fn event_update(
        &self,
        event: Arc<RwLock<EventInstance<DERControl>>>,
        status: EIStatus,
    ) -> ResponseStatus {
        match status {
            EIStatus::Scheduled => {
                println!("Received DERControl: {:?}", event.read().await.event());
            }
            EIStatus::Active => {
                println!("DERControl Started: {:?}", event.read().await.event());
            }
            EIStatus::Cancelled => {
                println!("DERControl Cancelled: {:?}", event.read().await.event());
            }
            EIStatus::Complete => {
                println!("DERControl Complete: {:?}", event.read().await.event());
            }
            EIStatus::CancelledRandom => {
                println!("DERControl Cancelled: {:?}", event.read().await.event());
            }
            EIStatus::Superseded => {
                println!("DERControl Started: {:?}", event.read().await.event());
            }
        };
        // TODO: This needs to call FS specific conversions
        // The returned ResponseStatus may differ depending on the client's requirements.
        status.into()
    }
}

// Example implementation of asynchronous polling
async fn poll_dcap(client: &Client) -> Result<()> {
    let dcap = client.get::<DeviceCapability>("/dcap").await?;
    let (tx, mut rx) = mpsc::channel::<DeviceCapability>(100);
    client
        .start_poll(
            dcap.href.unwrap(),
            Some(Uint32(1)),
            move |dcap: DeviceCapability| {
                let tx = tx.clone();
                async move { tx.send(dcap).await.unwrap() }
            },
        )
        .await;
    tokio::task::spawn(async move {
        // Whenever a poll is successful, print the resource
        while let Some(dcap) = rx.recv().await {
            println!("Updated DeviceCapability: {:?}", dcap);
        }
    });
    Ok(())
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
    // Setup a task to poll server device capability
    poll_dcap(&client)
        .await
        .map_err(|_| anyhow!("Failed to retrieve an initial instance of DCAP"))?;
    // All setup, run forever.
    notif_handle.await?
}
