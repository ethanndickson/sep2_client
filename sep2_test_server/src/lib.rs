use std::{
    future::Future,
    net::{self, SocketAddr},
    path::Path,
};

use anyhow::{anyhow, Result};
use hyper::{
    header::LOCATION, server::conn::Http, service::service_fn, Body, Method, Request, Response,
    StatusCode,
};
use openssl::ssl::{Ssl, SslAcceptor, SslAcceptorBuilder, SslFiletype, SslMethod, SslVerifyMode};

use sep2_common::examples::{
    DC_16_04_11, EDL_16_02_08, ED_16_01_08, ED_16_03_06, ER_16_04_06, FSAL_16_03_11, REG_16_01_10,
};
use tokio::net::TcpListener;
use tokio_openssl::SslStream;

type TlsServerConfig = SslAcceptorBuilder;
fn create_server_tls_config(
    cert_path: impl AsRef<Path>,
    pk_path: impl AsRef<Path>,
    rootca_path: impl AsRef<Path>,
) -> Result<TlsServerConfig> {
    let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls_server()).unwrap();
    log::debug!("Setting CipherSuite");
    builder.set_cipher_list("ECDHE-ECDSA-AES128-CCM8")?;
    log::debug!("Loading Certificate File");
    builder.set_certificate_file(cert_path, SslFiletype::PEM)?;
    log::debug!("Loading Private Key File");
    builder.set_private_key_file(pk_path, SslFiletype::PEM)?;
    log::debug!("Loading Certificate Authority File");
    builder.set_ca_file(rootca_path)?;
    log::debug!("Setting verification mode");
    builder.set_verify(SslVerifyMode::FAIL_IF_NO_PEER_CERT | SslVerifyMode::PEER);
    Ok(builder)
}

pub struct TestServer {
    addr: SocketAddr,
    cfg: TlsServerConfig,
}

impl TestServer {
    pub fn new(
        addr: impl net::ToSocketAddrs,
        cert_path: impl AsRef<Path>,
        pk_path: impl AsRef<Path>,
        rootca_path: impl AsRef<Path>,
    ) -> Result<Self> {
        let cfg = create_server_tls_config(cert_path, pk_path, rootca_path)?;
        Ok(TestServer {
            addr: addr
                .to_socket_addrs()?
                .next()
                .ok_or(anyhow!("Given server address did not yield a SocketAddr"))?,
            cfg,
        })
    }

    pub async fn run(self, shutdown: impl Future) -> Result<()> {
        tokio::pin!(shutdown);
        let acceptor = self.cfg.build();
        let listener = TcpListener::bind(self.addr).await?;
        let mut set = tokio::task::JoinSet::new();
        log::info!("TestServer: Listening on {}", self.addr);
        loop {
            // Accept TCP Connection
            let (stream, addr) = tokio::select! {
                _ = &mut shutdown => break,
                res = listener.accept() => match res {
                    Ok((s,a)) => (s,a),
                    Err(err) => {
                        log::error!("TestServer: Failed to accept connection: {err}");
                        continue;
                    }
                }
            };
            log::debug!("TestServer: Remote connecting from {}", addr);

            // Perform TLS handshake
            let ssl = Ssl::new(acceptor.context())?;
            let stream = SslStream::new(ssl, stream)?;
            let mut stream = Box::pin(stream);
            if let Err(e) = stream.as_mut().accept().await {
                log::error!("TestServer: Failed to perform TLS handshake: {e}");
                continue;
            }

            // Bind connection to service
            let service = service_fn(move |req| async move { router(req).await });
            set.spawn(async move {
                if let Err(err) = Http::new().serve_connection(stream, service).await {
                    log::error!("TestServer: Failed to handle connection: {err}");
                }
            });
        }
        // Wait for all connection handlers to finish
        log::debug!("TestServer: Attempting graceful shutdown");
        set.shutdown().await;
        log::info!("TestServer: Server has been shutdown.");
        Ok(())
    }
}

async fn router(req: Request<Body>) -> Result<Response<Body>> {
    log::info!("Incoming Request: {:?}", req);
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
        (&Method::PUT, "/edev/3") => {
            *response.status_mut() = StatusCode::NO_CONTENT;
        }
        (&Method::DELETE, "/edev/3") => {
            *response.status_mut() = StatusCode::NO_CONTENT;
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
        (&Method::POST, "/rsp") => {
            *response.status_mut() = StatusCode::CREATED;
            // Location header is unset in examples, but is technically always required by spec?
            // Client will handle missing location header regardless.
        }
        _ => {
            *response.status_mut() = StatusCode::NOT_FOUND;
        }
    };
    log::info!("Outgoing Response: {:?}", response);
    Ok(response)
}
