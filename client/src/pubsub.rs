use anyhow::Result;
use dashmap::DashMap;
use hyper::{server::conn::Http, service::service_fn, Body, Method, Request, Response};
use openssl::ssl::{Ssl, SslAcceptor};
use sep2_common::{deserialize, packages::pubsub::Notification, traits::SEResource};
use std::{future::Future, net::SocketAddr, pin::Pin, sync::Arc};
use tokio::net::TcpListener;
use tokio_openssl::SslStream;

use crate::client::SEPResponse;
use crate::tls::{create_server_tls_config, TlsServerConfig};

type RouteCallback = Box<
    dyn FnMut(&str) -> Pin<Box<dyn Future<Output = SEPResponse> + Send + Sync + 'static>>
        + Send
        + Sync
        + 'static,
>;

/// A lightweight
struct Router {
    routes: DashMap<String, RouteCallback>,
}

impl Router {
    fn new() -> Self {
        Router {
            routes: DashMap::new(),
        }
    }

    async fn router(&self, req: Request<Body>) -> Result<Response<Body>> {
        let path = req.uri().path().to_owned();
        match req.method() {
            &Method::POST => {
                let body = req.into_body();
                let bytes = hyper::body::to_bytes(body).await?;
                let xml = String::from_utf8(bytes.to_vec())?;
                match self.routes.get_mut(&path) {
                    Some(mut func) => {
                        let func = func.value_mut();
                        Ok(func(&xml).await.into())
                    }
                    None => Ok(SEPResponse::NotFound.into()),
                }
            }
            _ => Ok(SEPResponse::MethodNotAllowed("POST").into()),
        }
    }
}

/// A lightweight IEEE 2030.5 Server accepting a generic HTTP router.
/// For use in the system test server binary, and in the Client as the receiver for the subscription/notification mechanism
pub struct ClientNotifServer {
    addr: SocketAddr,
    cfg: Option<TlsServerConfig>,
    router: Router,
}

impl ClientNotifServer {
    pub fn new(addr: &str) -> Result<Self> {
        Ok(ClientNotifServer {
            addr: addr.parse()?,
            cfg: None,
            router: Router::new(),
        })
    }

    /// We assume IEEE 2030.5 Clients operating as aggregates will run the Client Notification Server
    /// behind an external reverse proxy. Thus, terminating TLS at the application is optional,
    /// and can be enabled by supplying certificates to this method.
    pub fn https(mut self, cert_path: &str, pk_path: &str) -> Result<Self> {
        let cfg = create_server_tls_config(cert_path, pk_path)?;
        self.cfg = Some(cfg);
        Ok(self)
    }

    /// Add a route to the server.
    /// Given:
    /// - A relative URI of the form "/foo"
    /// - A `FnMut1` callback accepting a [`Notification<T>`]`, where T is the expected [`SEResource`] on the route  
    ///
    /// [`SEResource`]: sep2_common::traits::SEResource
    pub fn add<F, T, R>(self, path: impl Into<String>, mut callback: F) -> Self
    where
        T: SEResource,
        F: FnMut(Notification<T>) -> R + Send + Sync + 'static,
        R: Future<Output = SEPResponse> + Send + Sync + 'static,
    {
        let path = path.into();
        let log_path = path.clone();
        let new: RouteCallback = Box::new(move |e| {
            let e = deserialize::<Notification<T>>(e);
            match e {
                Ok(resource) => {
                    log::debug!("NotifServer: Successfully deserialized a resource on {log_path}");
                    Box::pin(callback(resource))
                }
                Err(err) => {
                    log::error!("NotifServer: Failed to deserialize resource on {log_path}: {err}");
                    Box::pin(async { SEPResponse::BadRequest(None) })
                }
            }
        });
        self.router.routes.insert(path, new);
        self
    }

    pub async fn run(self, shutdown: impl Future) -> Result<()> {
        tokio::pin!(shutdown);
        let ssl_cfg: Option<SslAcceptor> = match self.cfg {
            Some(a) => Some(a.build()),
            None => {
                log::debug!("NotifServer: No SSL Configuration found, will serve using HTTP.");
                None
            }
        };
        let router = Arc::new(self.router);
        let listener = TcpListener::bind(self.addr).await?;
        let mut set = tokio::task::JoinSet::new();
        log::info!("NotifServer: Listening on {}", self.addr);
        loop {
            // Accept TCP Connection
            let (stream, addr) = tokio::select! {
                _ = &mut shutdown => break,
                res = listener.accept() => match res {
                    Ok((s,a)) => (s,a),
                    Err(err) => {
                        log::error!("NotifServer: Failed to accept connection: {err}");
                        continue;
                    }
                }
            };
            log::debug!("NotifServer: Remote connecting from {}", addr);

            // Bind connection to service
            let router = router.clone();
            let service = service_fn(move |req| {
                let handler = router.clone();
                async move { handler.router(req).await }
            });

            // Perform TLS handshake
            if let Some(acceptor) = &ssl_cfg {
                let ssl = Ssl::new(acceptor.context())?;
                let stream = SslStream::new(ssl, stream)?;
                let mut stream = Box::pin(stream);
                stream.as_mut().accept().await?;
                set.spawn(async move {
                    if let Err(err) = Http::new().serve_connection(stream, service).await {
                        log::error!("NotifServer: Failed to handle connection: {err}");
                    }
                });
            } else {
                set.spawn(async move {
                    if let Err(err) = Http::new().serve_connection(stream, service).await {
                        log::error!("NotifServer: Failed to handle connection: {err}");
                    }
                });
            }
        }
        // Wait for all connection handlers to finish
        log::debug!("NotifServer: Attempting graceful shutdown");
        set.shutdown().await;
        log::info!("NotifServer: Server has been shutdown.");
        Ok(())
    }
}
