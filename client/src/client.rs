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
use tokio::sync::RwLock;

use crate::tls::{create_client, create_client_tls_cfg, HTTPSClient};

#[cfg(feature = "der")]
use crate::time::current_time;
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
// This trait uses extra heap allocations while we await stable RPITIT (and eventually async fn with a send bound future)
#[async_trait::async_trait]
pub trait PollCallback<T: SEResource>: Clone + Send + Sync + 'static {
    async fn callback(&self, resource: T);
}

#[async_trait::async_trait]
impl<F, R, T: SEResource> PollCallback<T> for F
where
    F: Fn(T) -> R + Send + Sync + Clone + 'static,
    R: Future<Output = ()> + Send + 'static,
{
    async fn callback(&self, resource: T) {
        Box::pin(self(resource)).await
    }
}

type PollHandler =
    Box<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> + Send + Sync + 'static>;

struct PollTask {
    handler: PollHandler,
    interval: Duration,
    // Since poll intervals are duration based,
    // and not real-world timestamp based, we use [`Instant`]
    next: Instant,
}

impl PollTask {
    /// Run the stored handler, and increment the `next` Instant
    async fn execute(&mut self) {
        (self.handler)().await;
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
    http: HTTPSClient,
    polls: PollQueue,
}

impl Client {
    const DEFAULT_POLLRATE: Uint32 = Uint32(900);
    const DEFAULT_TICKRATE: Duration = Duration::from_secs(600);

    /// **TCP KeepAlive**:
    ///
    /// Pass the given value to `SO_KEEPALIVE`.
    ///
    /// **Tickrate**:
    ///
    /// Set how often the client background task should wakeup to check the polling queue.
    ///
    /// Defaults to 10 minutes, if this function is not called.
    ///
    /// If [`Client::start_poll`] is never called, this setting has no effect.
    pub fn new(
        server_addr: &str,
        cert_path: &str,
        pk_path: &str,
        tcp_keepalive: Option<Duration>,
        tickrate: Option<Duration>,
    ) -> Result<Self> {
        let cfg = create_client_tls_cfg(cert_path, pk_path)?;
        let out = Client {
            addr: server_addr.to_owned().into(),
            http: create_client(cfg, tcp_keepalive),
            polls: Arc::new(RwLock::new(BinaryHeap::new())),
        };
        tokio::spawn(
            out.clone()
                .poll_task(tickrate.unwrap_or(Self::DEFAULT_TICKRATE)),
        );
        Ok(out)
    }

    async fn poll_task(self, tickrate: Duration) {
        loop {
            tokio::time::sleep(tickrate).await;
            let mut polls = self.polls.write().await;
            while let Some(task) = polls.peek() {
                if task.next < Instant::now() {
                    let mut cur = polls.pop().unwrap();
                    cur.execute().await;
                    polls.push(cur);
                } else {
                    break;
                }
            }
        }
    }

    /// Retrieve the [`SEResource`] at the given path.
    ///
    /// Returns an error if the resource could not be retrieved or deserialized.
    pub async fn get<R: SEResource>(&self, path: &str) -> Result<R> {
        let uri: Uri = format!("{}{}", self.addr, path)
            .parse()
            .context("Failed to parse address")?;
        log::info!("Client: GET {} from {}", R::name(), uri);
        let req = Request::builder()
            .method(Method::GET)
            .header(ACCEPT, "application/sep+xml")
            .uri(uri)
            .body(Body::default())?;
        log::debug!("Client: Outgoing HTTP Request: {:?}", req);
        let res = self.http.request(req).await?;
        log::debug!("Client: Incoming HTTP Response: {:?}", res);
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
        log::info!("Client: DELETE at {}", uri);
        let req = Request::builder()
            .method(Method::DELETE)
            .uri(uri)
            .body(Body::empty())?;
        log::debug!("Client: Outgoing HTTP Request: {:?}", req);
        let res = self.http.request(req).await?;
        log::debug!("Client: Incoming HTTP Response: {:?}", res);
        // TODO: Improve error handling
        match res.status() {
            StatusCode::NO_CONTENT => Ok(()),
            StatusCode::BAD_REQUEST => bail!("400 Bad Request"),
            StatusCode::NOT_FOUND => bail!("404 Not Found"),
            _ => bail!("Unexpected HTTP response from server"),
        }
    }

    /// Begin polling the given route by performing GET requests on a regular interval. Passes the returned [`SEResource`] to the given callback.
    ///
    /// The callback will not be run if the GET request fails, or the resource cannot be deserialized.
    ///
    /// As per IEEE 2030.5, if a poll rate is not specified, a default of 900 seconds (15 minutes) is used.
    ///
    /// All poll events created can be forcibly run using [`Client::force_poll`], such as is required when reconnecting to the server after a period of connectivity loss.
    pub async fn start_poll<F, T>(
        &self,
        path: impl Into<String>,
        poll_rate: Option<Uint32>,
        callback: F,
    ) where
        T: SEResource,
        F: PollCallback<T>,
    {
        let client = self.clone();
        let path: String = path.into();
        let rate = poll_rate.unwrap_or(Self::DEFAULT_POLLRATE).get();
        let new: PollHandler = Box::new(move || {
            let path = path.clone();
            let client = client.clone();
            // Each time this closure is called, we need to produce a single future to return,
            // that future includes our callback, so we need to clone it every invocation of the poll callback.
            let callback = callback.clone();
            Box::pin(async move {
                match client.get::<T>(&path).await {
                    Ok(rsrc) => {
                        log::info!(
                            "Client: Scheduled poll for Resource {} successful.",
                            T::name()
                        );
                        callback.callback(rsrc).await;
                    }
                    Err(err) => {
                        log::warn!(
                            "Client: Scheduled poll for Resource {} at {} failed with reason {}. Retrying in {} seconds.",
                            T::name(),
                            &path,
                            err,
                            &rate
                        );
                        return;
                    }
                };
            })
        });
        let interval = Duration::from_secs(rate as u64);
        let poll = PollTask {
            handler: new,
            interval: interval,
            next: Instant::now() + interval,
        };
        self.polls.write().await.push(poll);
    }

    /// Forcibly poll & run the callbacks of all routes polled using [`Client::start_poll`]
    pub async fn force_poll(&self) {
        let mut polls = self.polls.write().await;
        while polls.peek().is_some() {
            let mut cur = polls.pop().unwrap();
            cur.execute().await;
            polls.push(cur);
        }
    }

    /// Cancel all poll tasks created using [`Client::start_poll`]
    pub async fn cancel_polls(&self) {
        self.polls.write().await.clear();
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
        log::debug!("Client: Outgoing HTTP Request: {:?}", req);
        let res = self.http.request(req).await?;
        log::debug!("Client: Incoming HTTP Response: {:?}", res);
        // TODO: Improve error handling
        match res.status() {
            // We leave the checking of the location header up to the client
            StatusCode::CREATED => {
                let loc = res
                    .headers()
                    .get(LOCATION)
                    .and_then(|h| h.to_str().ok())
                    .map(|r| r.to_string());
                Ok(SEPResponse::Created(loc))
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
        lfdi: HexBinary160,
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
                    "Client: DER response POST attempt failed with HTTP status code: {}",
                    e
                );
            }
            Err(e) => log::warn!(
                "Client: DER response POST attempt failed with reason: {}",
                e
            ),
            Ok(r @ (SEPResponse::Created(_) | SEPResponse::NoContent)) => {
                log::info!(
                    "Client: DER response POST attempt succeeded with reason: {}",
                    r
                )
            }
        }
    }
    #[cfg(feature = "der")]
    #[inline(always)]
    async fn do_der_response(
        &self,
        lfdi: HexBinary160,
        event: &DERControl,
        status: ResponseStatus,
    ) -> Result<SEPResponse> {
        // As per Table 27 - DER Column
        match (status, event.response_required) {
            (ResponseStatus::EventReceived, Some(rr))
                if rr.contains(ResponseRequired::MessageReceived) => {}
            (ResponseStatus::EventAcknowledge, Some(rr))
                if rr.contains(ResponseRequired::ResponseRequired) => {}
            (ResponseStatus::EventNoDisplay, _) => {
                bail!("Attempted to send a response unsupported by this function set.")
            }
            (_, Some(rr)) if rr.contains(ResponseRequired::SpecificResponse) => {}
            _ => bail!("Attempted to send a response for an event that did not require one."),
        };

        let resp = DERControlResponse {
            created_date_time: Some(current_time()),
            end_device_lfdi: lfdi,
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
    }
}
