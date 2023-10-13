use anyhow::{anyhow, bail, Context, Result};
use hyper::{
    header::{ACCEPT, ALLOW, CONTENT_LENGTH, CONTENT_TYPE, LOCATION},
    http::HeaderValue,
    Body, Method, Request, StatusCode, Uri,
};
use sep2_common::{
    deserialize,
    packages::{objects::Error, primitives::Uint32},
    serialize,
    traits::SEResource,
};
use std::{
    collections::BinaryHeap,
    fmt::Display,
    future::Future,
    pin::Pin,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{
    broadcast::{self, Sender},
    RwLock,
};

use crate::time::current_time;
use crate::tls::{create_client, create_client_tls_cfg, HTTPSClient};

#[cfg(feature = "der")]
use sep2_common::{
    packages::{
        der::DERControl,
        identification::{ResponseRequired, ResponseStatus},
        primitives::HexBinary160,
        response::DERControlResponse,
    },
    traits::SERespondableResource,
};

/// Possible HTTP Responses for a IEE 2030.5 Client to both send & receive.
pub enum SEPResponse {
    // HTTP 201 w/ Location header value, if it exists - 2030.5-2018 - 5.5.2.4
    Created(Option<String>),
    // HTTP 204 - 2030.5-2018 - 5.5.2.5
    NoContent,
    // HTTP 400 - 2030.5-2018 - 5.5.2.9
    BadRequest(Option<Error>),
    // HTTP 404 - 2030.5-2018 - 5.5.2.11
    NotFound,
    // HTTP 405 w/ Allow header value - 2030.5-2018 - 5.5.2.12
    MethodNotAllowed(&'static str),
}

impl Display for SEPResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SEPResponse::Created(loc) => {
                write!(
                    f,
                    "201 Created - Location Header {}",
                    match loc {
                        Some(loc) => loc,
                        None => "None",
                    }
                )
            }
            SEPResponse::NoContent => write!(f, "204 No Content"),
            SEPResponse::BadRequest(e) => match e {
                Some(e) => write!(f, "400 Bad Request - Error: {}", e),
                None => write!(f, "400 Bad Request"),
            },
            SEPResponse::NotFound => write!(f, "404 Not Found"),
            SEPResponse::MethodNotAllowed(allow) => {
                write!(f, "405 Method Not Allowed - Allow Header {}", allow)
            }
        }
    }
}

impl From<SEPResponse> for hyper::Response<Body> {
    fn from(value: SEPResponse) -> Self {
        let mut res = hyper::Response::new(Body::empty());
        match value {
            SEPResponse::Created(loc) => {
                *res.status_mut() = StatusCode::CREATED;
                if let Some(loc) = loc {
                    res.headers_mut().insert(LOCATION, loc.parse().unwrap());
                }
            }
            SEPResponse::NoContent => {
                *res.status_mut() = StatusCode::NO_CONTENT;
            }
            SEPResponse::BadRequest(_) => {
                *res.status_mut() = StatusCode::BAD_REQUEST;
            }
            SEPResponse::NotFound => {
                *res.status_mut() = StatusCode::NOT_FOUND;
            }
            SEPResponse::MethodNotAllowed(methods) => {
                *res.status_mut() = StatusCode::METHOD_NOT_ALLOWED;
                res.headers_mut()
                    .insert(ALLOW, HeaderValue::from_static(methods));
            }
        };
        res
    }
}

#[derive(Clone)]
enum PollTaskOld {
    ForceRun,
    Cancel,
}

type PollCallback = Box<
    dyn FnMut() -> Pin<Box<dyn Future<Output = ()> + Send + Sync + 'static>>
        + Send
        + Sync
        + 'static,
>;

struct PollTask {
    callback: PollCallback,
    interval: Duration,
    // Since poll intervals are duration based,
    // and not real-world timestamp based, we use [`Instant`]
    next: Instant,
}

impl PollTask {
    /// Run the store callback, and increment the `next` Instant
    async fn execute(&mut self) {
        (self.callback)().await;
        self.next = Instant::now() + self.interval;
    }
}

impl PartialEq for PollTask {
    fn eq(&self, other: &Self) -> bool {
        self.next == other.next
    }
}

impl Eq for PollTask {}

impl PartialOrd for PollTask {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PollTask {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.next.cmp(&other.next).reverse()
    }
}

type PollQueue = Arc<RwLock<BinaryHeap<PollTask>>>;

/// Represents an IEEE 2030.5 Client connection to a single server
///
/// Can be cloned cheaply as poll tasks, and the underlying `hyper` connection pool are shared between cloned clients.
#[derive(Clone)]
pub struct Client {
    addr: Arc<String>,
    // TODO: Move this into a cloneable inner
    http: HTTPSClient,
    broadcaster: Sender<PollTaskOld>,
    polls: PollQueue,
}

impl Client {
    const DEFAULT_POLLRATE: Uint32 = Uint32(900);

    pub fn new(
        server_addr: &str,
        cert_path: &str,
        pk_path: &str,
        tcp_keepalive: Option<Duration>,
    ) -> Result<Self> {
        let cfg = create_client_tls_cfg(cert_path, pk_path)?;
        let (tx, _) = broadcast::channel::<PollTaskOld>(1);
        Ok(Client {
            addr: server_addr.to_owned().into(),
            http: create_client(cfg, tcp_keepalive),
            broadcaster: tx,
            polls: Arc::new(RwLock::new(BinaryHeap::new())),
        })
    }

    /// Retrieve the [`SEResource`] at the given path.
    ///
    /// Returns an error if the resource could not be retrieved or deserialized.
    pub async fn get<R: SEResource>(&self, path: &str) -> Result<R> {
        let uri: Uri = format!("{}{}", self.addr, path)
            .parse()
            .context("Failed to parse address")?;
        log::info!("GET {} from {}", R::name(), uri);
        let req = Request::builder()
            .method(Method::GET)
            .header(ACCEPT, "application/sep+xml")
            .uri(uri)
            .body(Body::default())?;
        log::debug!("Outgoing HTTP Request: {:?}", req);
        let res = self.http.request(req).await?;
        log::debug!("Incoming HTTP Response: {:?}", res);
        // TODO: Improve error handling
        // TODO: Handle moved resources
        match res.status() {
            StatusCode::OK => (),
            StatusCode::NOT_FOUND => bail!("404 Not Found"),
            _ => bail!("Unexpected HTTP response from server"),
        }
        let body = hyper::body::to_bytes(res.into_body()).await?;
        let xml = String::from_utf8_lossy(&body);
        deserialize(&xml)
    }

    /// Update a [`SEResource`] at the given path.
    ///
    /// Returns an error if the server does not respond with 204 No Content or 201 Created.
    pub async fn post<R: SEResource>(&self, path: &str, resource: &R) -> Result<SEPResponse> {
        self.put_post(path, resource, Method::POST).await
    }

    /// Create a [`SEResource`] at the given path.
    ///
    /// Returns an error if the server does not respond with 204 No Content or 201 Created.
    pub async fn put<R: SEResource>(&self, path: &str, resource: &R) -> Result<SEPResponse> {
        self.put_post(path, resource, Method::PUT).await
    }

    /// Delete the [`SEResource`] at the given path.
    ///
    /// Returns an error if the server does not respond with 204 No Content.
    pub async fn delete(&self, path: &str) -> Result<()> {
        let uri: Uri = format!("{}{}", self.addr, path)
            .parse()
            .context("Failed to parse address")?;
        log::info!("DELETE at {}", uri);
        let req = Request::builder()
            .method(Method::DELETE)
            .uri(uri)
            .body(Body::empty())?;
        log::debug!("Outgoing HTTP Request: {:?}", req);
        let res = self.http.request(req).await?;
        log::debug!("Incoming HTTP Response: {:?}", res);
        // TODO: Improve error handling
        match res.status() {
            StatusCode::NO_CONTENT => Ok(()),
            StatusCode::BAD_REQUEST => bail!("400 Bad Request"),
            StatusCode::NOT_FOUND => bail!("404 Not Found"),
            _ => bail!("Unexpected HTTP response from server"),
        }
    }

    /// Begin polling the given route on a regular interval, passing the returned [`SEResource`] to the given callback.
    ///
    /// As per IEEE 2030.5, if a poll rate is not specified, a default of 900 seconds (15 minutes) is used.
    ///
    /// All poll events created can be forcibly run using [`Client::force_poll`], such as is required when reconnecting to the server after a period of connectivity loss.
    pub async fn start_poll<R: SEResource, F, Res>(
        &self,
        path: impl Into<String>,
        poll_rate: Option<Uint32>,
        mut callback: F,
    ) where
        R: SEResource,
        F: FnMut(R) -> Res + Send + Sync + 'static,
        Res: Future<Output = ()> + Send + Sync + 'static,
    {
        let client = self.clone();
        let path: String = path.into();
        let new: PollCallback = Box::new(move || {
            let path = path.clone();
            let client = client.clone();
            // TODO: Finish
            Box::pin(async { () })
        });
        // let client = self.clone();
        // let mut rx = self.broadcaster.subscribe();
        // tokio::task::spawn(async move {
        //     let mut next =
        //         current_time().get() + poll_rate.unwrap_or(Self::DEFAULT_POLLRATE).get() as i64;
        //     loop {
        //         tokio::select! {
        //             // Sleep in 30 second increments to account for sleepy devices
        //             _ = crate::time::sleep_until(next, Duration::from_secs(30)) => (),
        //             flag = rx.recv() => {
        //                 match flag {
        //                     Ok(PollTaskOld::ForceRun) => (),
        //                     Ok(PollTaskOld::Cancel) => {
        //                         log::warn!("Cancelling poll task for {} at {}", R::name(), path);
        //                         break;
        //                     }
        //                     _ => {
        //                         log::warn!("Poll task for {} at {} could not listen on it's broadcast receiver", R::name(), path);
        //                         break;
        //                     }

        //                 }
        //             }
        //         }

        //         let res = client.get::<R>(&path).await;
        //         match res {
        //             Ok(rsrc) => {
        //                 log::info!("Scheduled poll for Resource {} successful.", R::name());
        //                 callback(rsrc).await;
        //             }
        //             Err(_) => log::warn!(
        //                 "Scheduled poll for Resource {} at {} failed. Retrying in {} seconds.",
        //                 R::name(),
        //                 &path,
        //                 &poll_rate.unwrap_or(Self::DEFAULT_POLLRATE)
        //             ),
        //         }

        //         // Set next poll time
        //         next =
        //             current_time().get() + poll_rate.unwrap_or(Self::DEFAULT_POLLRATE).get() as i64;
        //     }
        // });
    }

    /// Forcibly poll & run the callbacks of all routes polled using [`Client::start_poll`]
    pub async fn force_poll(&self) {
        let _ = self.broadcaster.send(PollTaskOld::ForceRun);
    }

    /// Cancel all poll tasks created using [`Client::start_poll`]
    pub async fn cancel_polls(&self) {
        let _ = self.broadcaster.send(PollTaskOld::Cancel);
    }

    // Create a PUT or POST request
    async fn put_post<R: SEResource>(
        &self,
        path: &str,
        resource: &R,
        method: Method,
    ) -> Result<SEPResponse> {
        let uri: Uri = format!("{}{}", self.addr, path)
            .parse()
            .context("Failed to parse address")?;
        log::info!("POST {} to {}", R::name(), uri);
        let rsrce = serialize(resource)?;
        let rsrce_size = rsrce.as_bytes().len();
        let req = Request::builder()
            .method(method)
            .header(CONTENT_TYPE, "application/sep+xml")
            .header(CONTENT_LENGTH, rsrce_size)
            .uri(uri)
            .body(Body::from(rsrce))?;
        log::debug!("Outgoing HTTP Request: {:?}", req);
        let res = self.http.request(req).await?;
        log::debug!("Incoming HTTP Response: {:?}", res);
        // TODO: Improve error handling
        match res.status() {
            StatusCode::CREATED => {
                let loc = res
                    .headers()
                    .get(LOCATION)
                    .ok_or(anyhow!("201 Created - Missing Location Header"))?
                    .to_str()?
                    .to_string();
                Ok(SEPResponse::Created(Some(loc)))
            }
            StatusCode::NO_CONTENT => Ok(SEPResponse::NoContent),
            StatusCode::BAD_REQUEST => bail!("400 Bad Request"),
            StatusCode::NOT_FOUND => bail!("404 Not Found"),
            _ => bail!("Unexpected HTTP response from server"),
        }
    }

    #[cfg(feature = "der")]
    pub(crate) async fn send_der_response(
        &self,
        lfdi: Option<HexBinary160>,
        event: &DERControl,
        status: ResponseStatus,
    ) {
        match self.do_der_response(lfdi, event, status).await {
            Ok(
                e @ (SEPResponse::BadRequest(_)
                | SEPResponse::NotFound
                | SEPResponse::MethodNotAllowed(_)),
            ) => {
                log::warn!(
                    "DER response POST attempt failed with HTTP status code: {}",
                    e
                );
            }
            Err(e) => log::warn!("DER response POST attempt failed with reason: {}", e),
            Ok(r @ (SEPResponse::Created(_) | SEPResponse::NoContent)) => {
                log::info!("DER response POST attempt succeeded with reason: {}", r)
            }
        }
    }
    #[cfg(feature = "der")]
    #[inline(always)]
    async fn do_der_response(
        &self,
        lfdi: Option<HexBinary160>,
        event: &DERControl,
        status: ResponseStatus,
    ) -> Result<SEPResponse> {
        if matches!(status, ResponseStatus::EventReceived)
            && event
                .response_required
                .map(|rr| {
                    rr.contains(
                        ResponseRequired::MessageReceived | ResponseRequired::SpecificResponse,
                    )
                })
                .ok_or(anyhow!("Event does not contain a ResponseRequired field"))?
        {
            let resp = DERControlResponse {
                created_date_time: Some(current_time()),
                end_device_lfdi: lfdi.ok_or(anyhow!(
                    "Attempted to send DER response for EndDevice that does not have an LFDI"
                ))?,
                status: Some(status),
                subject: event.mrid,
                href: None,
            };
            self.post(
                event
                    .reply_to()
                    .ok_or(anyhow!("Event does not contain a ReplyTo field"))?,
                &resp,
            )
            .await
        } else {
            bail!("Attempted to send a response for an event that did not require one.")
        }
    }
}
