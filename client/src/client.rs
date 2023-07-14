use hyper::Uri;
use std::error::Error;

use crate::tls::{create_client, create_tls_config, HTTPSClient};

pub struct Client {
    addr: String,
    http: HTTPSClient,
}

impl Client {
    pub fn new(server_addr: &str, cert_path: &str, pk_path: &str) -> std::io::Result<Self> {
        let cfg = create_tls_config(cert_path, pk_path)?;
        Ok(Client {
            addr: server_addr.to_owned(),
            http: create_client(cfg),
        })
    }
    pub async fn get(&self, path: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
        let uri: Uri = format!("https://{}{}", self.addr, path).parse()?;
        let res = self.http.get(uri).await?;
        println!("status: {}", res.status());
        Ok(())
    }
}
