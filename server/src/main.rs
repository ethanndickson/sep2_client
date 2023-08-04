use anyhow::{Ok, Result};
use async_trait::async_trait;
use clap::Parser;
use client::pubsub::{ClientNotifServer, NotifHandler};
use common::packages::xsd::{DeviceCapability, EndDevice};
use common::serialize;
use hyper::{Body, Method, StatusCode};
use hyper::{Request, Response};
use log::info;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Port that the server should listen on
    port: i32,
    /// Path to a Server SSL Certificate
    cert: String,
    /// Path to the Server's SSL Private Key
    key: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let addr = format!("127.0.0.1:{}", args.port);
    let handler = TestHandler {};
    let server = ClientNotifServer::new(&addr, &args.cert, &args.key, handler).unwrap();
    info!("Server listening on {addr}");
    server.run().await
}

pub struct TestHandler {}

#[async_trait]
impl NotifHandler for TestHandler {
    async fn router(&self, req: Request<Body>) -> Result<Response<Body>> {
        let mut response = Response::new(Body::empty());
        match (req.method(), req.uri().path()) {
            (&Method::GET, "/dcap") => {
                *response.body_mut() = Body::from(serialize(DeviceCapability::default())?);
            }
            (&Method::GET, "/edev/1/") => {
                *response.body_mut() = Body::from(serialize(EndDevice::default())?);
            }
            _ => {
                *response.status_mut() = StatusCode::NOT_FOUND;
            }
        };
        Ok(response)
    }
}
