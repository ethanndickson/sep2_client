use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use client::{
    client::{Client, SepResponse},
    pubsub::{ClientNotifServer, NotifHandler},
};
use common::{
    deserialize,
    packages::{
        pubsub::Notification,
        xsd::{DeviceCapability, Reading},
    },
};
use tokio::sync::RwLock;
use typemap_rev::{TypeMap, TypeMapKey};

struct ReadingResource;
impl TypeMapKey for ReadingResource {
    type Value = Reading;
}

struct Handler {
    state: Arc<RwLock<TypeMap>>,
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
            Err(_) => SepResponse::BadRequest,
        }
    }
}

impl Default for Handler {
    fn default() -> Self {
        Self {
            state: Default::default(),
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
        "127.0.0.1:1337",
        "../certs/client_cert.pem",
        "../certs/client_private_key.pem",
        None,
    )?;
    println!("{:?}", client.get::<DeviceCapability>("/dcap").await?);
    // Join notif task
    notif_handle.await?
}
