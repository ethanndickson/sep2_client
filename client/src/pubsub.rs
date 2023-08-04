use std::net::SocketAddr;
use std::sync::Arc;

use crate::tls::{create_server_tls_config, TlsServerConfig};
use anyhow::Result;
use async_trait::async_trait;
use hyper::server::conn::Http;
use hyper::{service::service_fn, Request, Response};
use hyper::{Body, Method, StatusCode};
use log::info;
use openssl::ssl::Ssl;
use tokio::net::TcpListener;
use tokio_openssl::SslStream;

/// A lightweight IEEE 2030.5 Server accepting a generic HTTP router.
/// For use in the system test server binary, and in the Client as the receiver for the subscription/notification mechanism
pub struct ClientNotifServer<H: NotifHandler> {
    addr: SocketAddr,
    cfg: TlsServerConfig,
    handler: H,
    // TODO: Shutdown inlet & Panic outlet
}

impl<H: NotifHandler> ClientNotifServer<H> {
    pub fn new(addr: &str, cert_path: &str, pk_path: &str, handler: H) -> Result<Self> {
        let cfg = create_server_tls_config(cert_path, pk_path)?;
        Ok(ClientNotifServer {
            addr: addr.parse()?,
            cfg,
            handler,
        })
    }

    pub async fn run(self) -> Result<()> {
        info!("NotifServer listening on {}", self.addr);
        let listener = TcpListener::bind(self.addr).await?;
        let acceptor = self.cfg.build();
        let handler = Arc::new(self.handler);
        loop {
            // Accept TCP Connection
            let (stream, _) = listener.accept().await?;
            let ssl = Ssl::new(acceptor.context())?;
            let stream = SslStream::new(ssl, stream)?;
            let mut stream = Box::pin(stream);
            // Perform TLS handshake
            stream.as_mut().accept().await?;

            // Bind connection to service
            let handler = handler.clone();
            let service = service_fn(move |req| {
                let handler = handler.clone();
                async move { handler.router(req).await }
            });
            tokio::task::spawn(async move {
                let _ = Http::new().serve_connection(stream, service).await;
            });
        }
    }
}

#[async_trait]
pub trait NotifHandler: Send + Sync + 'static {
    /// Default router when server is used to receive notifications
    async fn router(&self, req: Request<Body>) -> Result<Response<Body>> {
        let mut response = Response::new(Body::empty());
        match (req.method(), req.uri().path()) {
            (&Method::POST, "/note") => {
                *response.status_mut() = StatusCode::CREATED;
            }
            (_, "/note") => {
                *response.status_mut() = StatusCode::METHOD_NOT_ALLOWED;
            }
            _ => {
                *response.status_mut() = StatusCode::NOT_FOUND;
            }
        };
        Ok(response)
    }

    /// Function to be called in router to filter incoming notifications
    #[allow(unused_variables)]
    async fn notif_handler(&self, resource_name: &str, resource: &str) -> Result<()> {
        Ok(())
    }
}
