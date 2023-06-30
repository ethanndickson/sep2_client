use std::error::Error;
use std::net::SocketAddr;

use hyper::server::conn::Http;
use hyper::{service::service_fn, Request, Response};
use hyper::{Body, Method, StatusCode};
use openssl::ssl::Ssl;
use tokio::net::TcpListener;
use tokio_openssl::SslStream;

use crate::tls::{create_tls_config, TlsConfig};

pub struct Server {
    addr: SocketAddr,
    cfg: TlsConfig,
}

impl Server {
    pub fn new(
        addr: &str,
        cert_path: &str,
        pk_path: &str,
    ) -> Result<Self, Box<dyn Error + Send + Sync>> {
        let cfg = create_tls_config(addr, cert_path, pk_path)?;
        Ok(Server {
            addr: addr.parse()?,
            cfg,
        })
    }

    pub async fn run(self) -> Result<(), Box<dyn Error + Send + Sync>> {
        let listener = TcpListener::bind(self.addr).await?;
        let acceptor = self.cfg.build();

        loop {
            // Accept TCP Connection
            let (stream, _) = listener.accept().await?;
            let ssl = Ssl::new(acceptor.context())?;
            let stream = SslStream::new(ssl, stream)?;
            let mut stream = Box::pin(stream);
            // Perform TLS handshake
            stream.as_mut().accept().await?;
            // Bind connection to a service
            tokio::task::spawn(
                async move { Http::new().serve_connection(stream, service_fn(router)) },
            );
        }
    }
}

async fn router(req: Request<Body>) -> Result<Response<Body>, hyper::Error> {
    let mut response = Response::new(Body::empty());
    match (req.method(), req.uri().path()) {
        (&Method::GET, "/") => {
            *response.body_mut() = Body::from("Hello World!\n");
        }
        (&Method::POST, "/echo") => {
            *response.body_mut() = req.into_body();
        }
        _ => {
            *response.status_mut() = StatusCode::NOT_FOUND;
        }
    };

    Ok(response)
}
