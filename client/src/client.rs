use anyhow::Result;
use common::{deserialize, packages::traits::SEResource, serialize};
use hyper::{Body, Method, Request, Uri};

use crate::tls::{create_client, create_client_tls_cfg, HTTPSClient};
pub struct Client {
    addr: String,
    http: HTTPSClient,
}

impl Client {
    pub fn new(server_addr: &str, cert_path: &str, pk_path: &str) -> Result<Self> {
        let cfg = create_client_tls_cfg(cert_path, pk_path)?;
        Ok(Client {
            addr: server_addr.to_owned(),
            http: create_client(cfg),
        })
    }
    pub async fn get<R: SEResource>(&self, path: &str) -> Result<R> {
        let uri: Uri = format!("https://{}{}", self.addr, path).parse()?;
        let req = Request::builder()
            .method(Method::GET)
            .uri(uri)
            .body(Body::default())?;
        let res = self.http.request(req).await?;
        let body = hyper::body::to_bytes(res.into_body()).await?;
        let xml = String::from_utf8_lossy(&body);
        deserialize(&xml)
    }
}
