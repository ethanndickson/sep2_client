use anyhow::Result;
use async_trait::async_trait;
use client::{
    client::Client,
    subscription::{ClientNotifServer, NotifHandler},
};
use common::packages::xsd::DeviceCapability;

struct Handler {}
#[async_trait]
impl NotifHandler for Handler {
    async fn notif_handler(&self, name: &str, resource: &str) -> Result<()> {
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let notifs = ClientNotifServer::new(
        "127.0.0.1:1338",
        "../certs/client_cert.pem",
        " ../certs/client_private_key.pem",
        Handler {},
    )?;
    tokio::task::spawn(notifs.run());
    let client = Client::new(
        "127.0.0.1:1337",
        "../certs/client_cert.pem",
        "../certs/client_private_key.pem",
    )?;
    println!("{:?}", client.get::<DeviceCapability>("/").await?);
    Ok(())
}
