use anyhow::{Ok, Result};
use async_trait::async_trait;
use clap::Parser;
use client::pubsub::{ClientNotifServer, NotifHandler};
use common::examples::{
    DC_16_04_11, EDL_16_02_08, ED_16_01_08, ED_16_03_06, ER_16_04_06, FSAL_16_03_11, REG_16_01_10,
};
use common::packages::xsd::DeviceCapability;
use common::serialize;
use hyper::header::LOCATION;
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
        println!("Incoming Request: {:?}", req);
        let mut response = Response::new(Body::empty());
        match (req.method(), req.uri().path()) {
            (&Method::GET, "/dcap") => {
                *response.body_mut() = Body::from(DC_16_04_11);
            }
            (&Method::GET, "/edev") => {
                *response.body_mut() = Body::from(EDL_16_02_08);
            }
            (&Method::POST, "/edev") => {
                *response.status_mut() = StatusCode::CREATED;
                response
                    .headers_mut()
                    .insert(LOCATION, "/edev/4".parse().unwrap());
            }
            (&Method::GET, "/edev/3") => {
                *response.body_mut() = Body::from(ED_16_01_08);
            }
            (&Method::GET, "/edev/4/fsal") => {
                *response.body_mut() = Body::from(FSAL_16_03_11);
            }
            (&Method::GET, "/edev/4") => {
                *response.body_mut() = Body::from(ED_16_03_06);
            }
            (&Method::GET, "/edev/5") => {
                *response.body_mut() = Body::from(ER_16_04_06);
            }
            (&Method::GET, "/edev/3/reg") => {
                *response.body_mut() = Body::from(REG_16_01_10);
            }
            _ => {
                *response.status_mut() = StatusCode::NOT_FOUND;
            }
        };
        println!("Outgoing Response: {:?}", response);
        Ok(response)
    }
}
