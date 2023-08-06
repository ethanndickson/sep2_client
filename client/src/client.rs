use anyhow::{anyhow, bail, Ok, Result};
use common::{deserialize, packages::traits::SEResource, serialize};
use hyper::{
    header::{ACCEPT, CONTENT_LENGTH, CONTENT_TYPE, LOCATION},
    Body, Method, Request, StatusCode, Uri,
};
use log::{debug, info};

use crate::tls::{create_client, create_client_tls_cfg, HTTPSClient};

pub enum SepResponse {
    // HTTP 201 w/ Location header value
    Created(String),
    // HTTP 204
    NoContent,
}

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
        info!("GET {} from {}", R::name(), uri);
        let req = Request::builder()
            .method(Method::GET)
            .header(ACCEPT, "application/sep+xml")
            .uri(uri)
            .body(Body::default())?;
        debug!("Outgoing HTTP Request: {:?}", req);
        let res = self.http.request(req).await?;
        debug!("Incoming HTTP Response: {:?}", res);
        // TODO: Improve erorr handling
        match res.status() {
            StatusCode::OK => (),
            StatusCode::NOT_FOUND => bail!("404 Not Found"),
            _ => bail!("Unexpected HTTP response from server"),
        }
        let body = hyper::body::to_bytes(res.into_body()).await?;
        let xml = String::from_utf8_lossy(&body);
        deserialize(&xml)
    }

    pub async fn post<R: SEResource>(&self, path: &str, resource: &R) -> Result<SepResponse> {
        self.put_post(path, resource, Method::POST).await
    }

    pub async fn delete(&self, path: &str) -> Result<()> {
        let uri: Uri = format!("https://{}{}", self.addr, path).parse()?;
        info!("DELETE at {}", uri);
        let req = Request::builder()
            .method(Method::DELETE)
            .uri(uri)
            .body(Body::empty())?;
        debug!("Outgoing HTTP Request: {:?}", req);
        let res = self.http.request(req).await?;
        debug!("Incoming HTTP Response: {:?}", res);
        // TODO: Improve erorr handling
        match res.status() {
            StatusCode::NO_CONTENT => Ok(()),
            StatusCode::BAD_REQUEST => bail!("400 Bad Request"),
            StatusCode::NOT_FOUND => bail!("404 Not Found"),
            _ => bail!("Unexpected HTTP response from server"),
        }
    }

    pub async fn put<R: SEResource>(&self, path: &str, resource: &R) -> Result<SepResponse> {
        self.put_post(path, resource, Method::PUT).await
    }

    // Create a PUT or POST request
    async fn put_post<R: SEResource>(
        &self,
        path: &str,
        resource: &R,
        method: Method,
    ) -> Result<SepResponse> {
        let uri: Uri = format!("https://{}{}", self.addr, path).parse()?;
        info!("POST {} to {}", R::name(), uri);
        let rsrce = serialize(resource)?;
        let rsrce_size = rsrce.as_bytes().len();
        let req = Request::builder()
            .method(method)
            .header(CONTENT_TYPE, "application/sep+xml")
            .header(CONTENT_LENGTH, rsrce_size)
            .uri(uri)
            .body(Body::from(rsrce))?;
        debug!("Outgoing HTTP Request: {:?}", req);
        let res = self.http.request(req).await?;
        debug!("Incoming HTTP Response: {:?}", res);
        // TODO: Improve erorr handling
        match res.status() {
            StatusCode::CREATED => {
                let loc = res
                    .headers()
                    .get(LOCATION)
                    .ok_or(anyhow!("201 Created - Missing Location Header"))?
                    .to_str()?
                    .to_string();
                Ok(SepResponse::Created(loc))
            }
            StatusCode::NO_CONTENT => Ok(SepResponse::NoContent),
            StatusCode::BAD_REQUEST => bail!("400 Bad Request"),
            StatusCode::NOT_FOUND => bail!("404 Not Found"),
            _ => bail!("Unexpected HTTP response from server"),
        }
    }
}
