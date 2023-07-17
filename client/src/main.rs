use common::packages::xsd::DeviceCapability;

use crate::client::Client;
use std::error::Error;
mod client;
mod tls;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let client = Client::new(
        "127.0.0.1:1337",
        "../certs/client_cert.pem",
        "../certs/client_private_key.pem",
    )?;
    println!("{:?}", client.get::<DeviceCapability>("/").await?);
    Ok(())
}
