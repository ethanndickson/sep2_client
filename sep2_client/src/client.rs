//! IEEE 2030.5 Client Core Functionality

use anyhow::{anyhow, bail, Context, Result};
use httpdate::fmt_http_date;
use hyper::{
    header::{ACCEPT, ALLOW, CONTENT_LENGTH, CONTENT_TYPE, DATE, LOCATION},
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
    path::Path,
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

use crate::{
    time::{current_time_with_offset, SEPTime},
    tls::{create_client, create_client_tls_cfg, create_http_client, ClientInner},
};

#[cfg(feature = "event")]
use sep2_common::{
    packages::identification::{ResponseRequired, ResponseStatus},
    traits::SERespondableResource,
};

#[cfg(any(feature = "messaging", feature = "der", feature = "pricing"))]
use sep2_common::packages::primitives::HexBinary160;

#[cfg(feature = "der")]
use sep2_common::packages::{der::DERControl, response::DERControlResponse};

#[cfg(feature = "messaging")]
use sep2_common::packages::{messaging::TextMessage, response::TextResponse};

#[cfg(feature = "drlc")]
use {
    crate::device::SEDevice,
    sep2_common::packages::{drlc::EndDeviceControl, response::DrResponse},
};

#[cfg(feature = "pricing")]
use sep2_common::packages::{pricing::TimeTariffInterval, response::PriceResponse};

/// Possible HTTP Responses for a IEE 2030.5 Client to both send & receive.
pub enum SEPResponse {
    /// HTTP 201 w/ Location header value, if it exists - 2030.5-2018 - 5.5.2.4
    Created(Option<String>),
    /// HTTP 204 - 2030.5-2018 - 5.5.2.5
    NoContent,
    /// HTTP 400 - 2030.5-2018 - 5.5.2.9
    BadRequest(Option<Error>),
    /// HTTP 404 - 2030.5-2018 - 5.5.2.11
    NotFound,
    /// HTTP 405 w/ Allow header value - 2030.5-2018 - 5.5.2.12
    MethodNotAllowed(String),
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

impl TryFrom<SEPResponse> for hyper::Response<Body> {
    type Error = anyhow::Error;

    fn try_from(value: SEPResponse) -> Result<Self, Self::Error> {
        let mut res = hyper::Response::new(Body::empty());
        res.headers_mut().insert(
            DATE,
            fmt_http_date(current_time_with_offset().into())
                .parse()
                .context("Guaranteed parse somehow failed")?,
        );
        match value {
            SEPResponse::Created(loc) => {
                *res.status_mut() = StatusCode::CREATED;
                if let Some(loc) = loc {
                    res.headers_mut().insert(
                        LOCATION,
                        loc.parse().context("Failed to set LOCATION header")?,
                    );
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
                res.headers_mut().insert(
                    ALLOW,
                    HeaderValue::try_from(methods).context("Failed to set ALLOW header")?,
                );
            }
        };
        Ok(res)
    }
}

// async `TryFrom<Response<Body>> for SEPResponse`` implementation
async fn into_sepresponse(res: hyper::Response<Body>) -> Result<SEPResponse> {
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
        StatusCode::BAD_REQUEST => Ok(SEPResponse::BadRequest(
            hyper::body::to_bytes(res.into_body())
                .await
                .ok()
                .and_then(|b| {
                    let out = String::from_utf8_lossy(&b);
                    deserialize(&out).ok()
                }),
        )),
        StatusCode::NOT_FOUND => Ok(SEPResponse::NotFound),
        StatusCode::METHOD_NOT_ALLOWED => {
            let loc = res
                .headers()
                .get(ALLOW)
                .and_then(|h| h.to_str().ok())
                .map(|r| r.to_string())
                .context("Failed to extract expected ALLOW header from Response")?;
            Ok(SEPResponse::MethodNotAllowed(loc))
        }
        _ => Err(anyhow!("Unexpected HTTP response from server")),
    }
}

/// A trait implemented by types that can be used as a poll callback by [`Client::start_poll`].
pub trait PollCallback<T: SEResource>: Clone + Send + Sync + 'static {
    fn callback(&self, resource: T) -> impl Future<Output = ()> + Send;
}

/// Automatically implemented for all [`Fn`] with a matching function signature.
impl<F, R, T: SEResource> PollCallback<T> for F
where
    F: Fn(T) -> R + Send + Sync + Clone + 'static,
    R: Future<Output = ()> + Send + 'static,
{
    fn callback(&self, resource: T) -> impl Future<Output = ()> + Send {
        self(resource)
    }
}

type PollHandler = Box<
    dyn Fn(Duration) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> + Send + Sync + 'static,
>;

/// Identifies a poll job created by [`Client::start_poll`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PollId(
    // A u64 for an id means it is virtually impossible to run out of ids, even
    // if the client is spin-locking and allocating new ids constantly.
    u64,
);

struct PollJob {
    id: PollId,
    handler: PollHandler,
    interval: Duration,
    // Since poll intervals are duration based,
    // and not real-world timestamp based, we use [`Instant`]
    next: Instant,
}

impl PollJob {
    /// Run the stored handler, and increment the `next` Instant
    async fn execute(&mut self) {
        tokio::spawn((self.handler)(self.interval));
        self.next = Instant::now() + self.interval;
    }

    /// Updates the interval value and potentially updates the `next` value to
    /// whichever is earlier of a) the old `next` value and b) the current time
    /// advanced by the new interval.
    fn update_interval(&mut self, new_interval: Duration) {
        self.interval = new_interval;
        let new_interval_next = Instant::now() + new_interval;
        if self.next > new_interval_next {
            self.next = new_interval_next;
        }
    }
}

impl PartialEq for PollJob {
    fn eq(&self, other: &Self) -> bool {
        self.next == other.next
    }
}

impl Eq for PollJob {}

impl PartialOrd for PollJob {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PollJob {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.next.cmp(&other.next).reverse()
    }
}

type PollQueue = Arc<Mutex<BinaryHeap<PollJob>>>;

/// Represents an IEEE 2030.5 Client connection to a single server
///
/// Can be cloned cheaply as poll tasks, and the underlying `hyper` connection pool are shared between cloned clients.
#[derive(Clone)]
pub struct Client {
    addr: Arc<String>,
    inner: ClientInner,
    polls: PollQueue,
    poll_id_counter: Arc<AtomicU64>,
}

impl Client {
    const DEFAULT_POLLRATE: Uint32 = Uint32(900);
    const DEFAULT_TICKRATE: Duration = Duration::from_secs(600);

    /// Construct an IEEE 2030.5 Client instance that uses HTTP
    ///
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
        tcp_keepalive: Option<Duration>,
        tickrate: Option<Duration>,
    ) -> Result<Self> {
        let out = Client {
            addr: server_addr.to_owned().into(),
            inner: ClientInner::Http(create_http_client(tcp_keepalive)),
            polls: Arc::new(Mutex::new(BinaryHeap::new())),
            poll_id_counter: Arc::new(AtomicU64::new(1)),
        };
        tokio::spawn(
            out.clone()
                .poll_task(tickrate.unwrap_or(Self::DEFAULT_TICKRATE)),
        );
        Ok(out)
    }

    /// Construct an IEEE 2030.5 Client instance that uses HTTPS
    pub fn new_https(
        server_addr: &str,
        cert_path: impl AsRef<Path>,
        pk_path: impl AsRef<Path>,
        rootca_path: impl AsRef<Path>,
        tcp_keepalive: Option<Duration>,
        tickrate: Option<Duration>,
    ) -> Result<Self> {
        let cfg = create_client_tls_cfg(cert_path, pk_path, rootca_path)?;
        let out = Client {
            addr: server_addr.to_owned().into(),
            inner: ClientInner::Https(create_client(cfg, tcp_keepalive)),
            polls: Arc::new(Mutex::new(BinaryHeap::new())),
            poll_id_counter: Arc::new(AtomicU64::new(1)),
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
            let mut polls = self.polls.lock().await;
            while let Some(task) = polls.peek() {
                if task.next < Instant::now() {
                    // unwrap trivially safe
                    let mut cur = polls.pop().unwrap();
                    cur.execute().await;
                    polls.push(cur);
                } else {
                    break;
                }
            }
        }
    }

    fn next_poll_id(&self) -> PollId {
        PollId(self.poll_id_counter.fetch_add(1, Ordering::Relaxed))
    }

    /// Retrieve the [`SEResource`] at the given relative path.
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
            .header(DATE, fmt_http_date(current_time_with_offset().into()))
            .uri(uri)
            .body(Body::default())?;
        log::debug!("Client: Outgoing HTTP Request: {:?}", req);
        let res = self.inner.request(req).await?;
        log::debug!("Client: Incoming HTTP Response: {:?}", res);
        // TODO: Handle moved resources - implement HTTP redirects
        match res.status() {
            StatusCode::OK => (),
            e => bail!("Unexpected HTTP response from server: {}", e),
        }
        let body = hyper::body::to_bytes(res.into_body()).await?;
        let xml = String::from_utf8_lossy(&body);
        deserialize(&xml)
    }

    /// Update a [`SEResource`] at the given relative path.
    ///
    /// Returns an error if the server does not respond with 204 No Content or 201 Created.
    pub async fn post<R: SEResource>(&self, path: &str, resource: &R) -> Result<SEPResponse> {
        let path = format!("{}{}", self.addr, path);
        self.put_post(
            path.parse().context("Failed to parse address")?,
            resource,
            Method::POST,
            current_time_with_offset(),
        )
        .await
    }

    /// Create a [`SEResource`] at the given relative path.
    ///
    /// Returns an error if the server does not respond with 204 No Content or 201 Created.
    pub async fn put<R: SEResource>(&self, path: &str, resource: &R) -> Result<SEPResponse> {
        let path = format!("{}{}", self.addr, path);
        self.put_post(
            path.parse().context("Failed to parse address")?,
            resource,
            Method::PUT,
            current_time_with_offset(),
        )
        .await
    }

    /// Delete the [`SEResource`] at the given relative path.
    ///
    /// Returns an error if the server does not respond with 204 No Content.
    pub async fn delete(&self, path: &str) -> Result<SEPResponse> {
        let uri: Uri = format!("{}{}", self.addr, path)
            .parse()
            .context("Failed to parse address")?;
        log::info!("Client: DELETE at {}", uri);
        let req = Request::builder()
            .method(Method::DELETE)
            .header(DATE, fmt_http_date(current_time_with_offset().into()))
            .uri(uri)
            .body(Body::empty())?;
        log::debug!("Client: Outgoing HTTP Request: {:?}", req);
        let res = self.inner.request(req).await?;
        log::debug!("Client: Incoming HTTP Response: {:?}", res);
        into_sepresponse(res).await
    }

    /// Begin polling the given route by performing GET requests on a regular interval. Passes the returned [`SEResource`] to the given callback.
    ///
    /// Returns an id for the poll job that can be used to cancel or update it.
    ///
    /// The callback will not be run if the GET request fails, or the resource cannot be deserialized.
    ///
    /// As per IEEE 2030.5, if a poll rate is not specified, a default of 900 seconds (15 minutes) is used.
    ///
    /// All poll events created can be forcibly run using [`Client::force_polls`], such as is required when reconnecting to the server after a period of connectivity loss.
    pub async fn start_poll<T>(
        &self,
        path: impl Into<String>,
        poll_rate: Option<Uint32>,
        callback: impl PollCallback<T>,
    ) -> PollId
    where
        T: SEResource,
    {
        let new: PollHandler = Box::new({
            let client = self.clone();
            let path: String = path.into();
            move |interval| {
                Box::pin({
                    let path = path.clone();
                    let client = client.clone();
                    // Each time this closure is called, we need to produce a single future to return,
                    // that future includes our callback, so we need to clone it every invocation of the poll callback.
                    let callback = callback.clone();
                    async move {
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
                            interval.as_secs()
                        );
                            }
                        };
                    }
                })
            }
        });
        let poll_rate = poll_rate.unwrap_or(Self::DEFAULT_POLLRATE).get();
        let interval = Duration::from_secs(poll_rate as u64);
        let poll_id = self.next_poll_id();
        let poll = PollJob {
            id: poll_id,
            handler: new,
            interval,
            next: Instant::now() + interval,
        };
        self.polls.lock().await.push(poll);
        poll_id
    }

    /// Forcibly poll & run the callbacks of all routes polled using [`Client::start_poll`]
    pub async fn force_polls(&self) {
        let mut polls = self.polls.lock().await;
        while polls.peek().is_some() {
            // unwrap trivially safe
            let mut cur = polls.pop().unwrap();
            cur.execute().await;
            polls.push(cur);
        }
    }

    /// Cancel all poll tasks created using [`Client::start_poll`]
    pub async fn cancel_polls(&self) {
        self.polls.lock().await.clear();
    }

    /// Cancel a poll given a poll id. If no poll with the given id exists, no change occurs.
    pub async fn cancel_poll(&self, poll_id: PollId) {
        self.polls.lock().await.retain(|poll| poll.id != poll_id);
    }

    /// Changes a poll's interval to a new value. If no poll with the given id
    /// exists, no change occurs.
    ///
    /// The poll's next polling time will either remain its previous value, or
    /// be changed to be `poll_rate` seconds from now, whichever is earlier.
    pub async fn update_poll_rate(&self, poll_id: PollId, poll_rate: Uint32) {
        // This is a little awkward because a BinaryHeap can't pop a single item
        // at random. Instead we must pull all jobs out of the heap and then
        // reconstruct it. We are also not allowed to update the `next` value in
        // place, as that would invalidate the ordering of the BinaryHeap.

        let interval = Duration::from_secs(u64::from(poll_rate.get()));

        let mut polls = self.polls.lock().await;
        let mut items: Vec<_> = polls.drain().collect();
        if let Some(poll) = items.iter_mut().find(|poll| poll.id == poll_id) {
            poll.update_interval(interval);
        }
        *polls = BinaryHeap::from(items);
    }

    // Create a PUT or POST request
    async fn put_post<R: SEResource>(
        &self,
        abs_path: Uri,
        resource: &R,
        method: Method,
        time: SEPTime,
    ) -> Result<SEPResponse> {
        log::info!("POST {} to {}", R::name(), abs_path);
        let rsrce = serialize(resource)?;
        let rsrce_size = rsrce.len();
        let req = Request::builder()
            .method(method)
            .header(CONTENT_TYPE, "application/sep+xml")
            .header(CONTENT_LENGTH, rsrce_size)
            .header(DATE, fmt_http_date(time.into()))
            .uri(abs_path)
            .body(Body::from(rsrce))?;
        log::debug!("Client: Outgoing HTTP Request: {:?}", req);
        let res = self.inner.request(req).await?;
        log::debug!("Client: Incoming HTTP Response: {:?}", res);
        into_sepresponse(res).await
    }

    #[cfg(feature = "messaging")]
    pub async fn send_msg_response(
        &self,
        lfdi: HexBinary160,
        event: &TextMessage,
        status: ResponseStatus,
        time: SEPTime,
    ) -> Result<SEPResponse> {
        // As per Messaging in Table 27
        match (status, event.response_required) {
            (ResponseStatus::EventReceived, Some(rr))
                if rr.contains(ResponseRequired::MessageReceived) => {}
            (ResponseStatus::EventStarted, Some(rr))
                if rr.contains(ResponseRequired::SpecificResponse) => {}
            (ResponseStatus::EventCompleted, Some(rr))
                if rr.contains(ResponseRequired::SpecificResponse) => {}
            (ResponseStatus::EventSuperseded, Some(rr))
                if rr.contains(ResponseRequired::SpecificResponse) => {}
            (ResponseStatus::EventAcknowledge, Some(rr))
                if rr.contains(ResponseRequired::ResponseRequired) => {}
            (ResponseStatus::EventNoDisplay, Some(rr))
                if rr.contains(ResponseRequired::SpecificResponse) => {}
            (ResponseStatus::EventAbortedServer, Some(rr))
                if rr.contains(ResponseRequired::SpecificResponse) => {}
            (ResponseStatus::EventAbortedProgram, Some(rr))
                if rr.contains(ResponseRequired::SpecificResponse) => {}
            (ResponseStatus::EventExpired, Some(rr))
                if rr.contains(ResponseRequired::SpecificResponse) => {}
            _ => bail!("Attempted to send a response for an event where one was not required, either due to it's status or the event's responseRequired field.")
        };
        let resp = TextResponse {
            created_date_time: Some(time.into()),
            end_device_lfdi: lfdi,
            status: Some(status),
            subject: event.mrid,
            href: None,
        };
        self.put_post(
            event
                .reply_to()
                .context("Event does not contain a ReplyTo Field")?
                .parse()
                .context("Failed to parse ReplyTo Field")?,
            &resp,
            Method::POST,
            time,
        )
        .await
    }

    #[cfg(feature = "der")]
    pub async fn send_der_response(
        &self,
        lfdi: HexBinary160,
        event: &DERControl,
        status: ResponseStatus,
        time: SEPTime,
    ) -> Result<SEPResponse> {
        // As per Table 27 - DER Column

        match (status, event.response_required) {
            (ResponseStatus::EventReceived, Some(rr))
                if rr.contains(ResponseRequired::MessageReceived) => {}
            (ResponseStatus::EventAcknowledge, Some(rr))
                if rr.contains(ResponseRequired::ResponseRequired) => {}
            (ResponseStatus::EventNoDisplay, _) => {
                bail!("Attempted to send a response for an event where one was not required, either due to it's status or the event's responseRequired field.")
            }
            (_, Some(rr)) if rr.contains(ResponseRequired::SpecificResponse) => {}
            _ => bail!("Attempted to send a response for an event where one was not required, either due to it's status or the event's responseRequired field."),
        };

        let resp = DERControlResponse {
            created_date_time: Some(time.into()),
            end_device_lfdi: lfdi,
            status: Some(status),
            subject: event.mrid,
            href: None,
        };
        self.put_post(
            event
                .reply_to()
                .context("Event does not contain a ReplyTo Field")?
                .parse()
                .context("Failed to parse ReplyTo Field")?,
            &resp,
            Method::POST,
            time,
        )
        .await
    }

    #[cfg(feature = "drlc")]
    pub async fn send_drlc_response(
        &self,
        device: &SEDevice,
        event: &EndDeviceControl,
        status: ResponseStatus,
        time: SEPTime,
    ) -> Result<SEPResponse> {
        // As per Table 27 - DRLC Column

        match (status, event.response_required) {
            (ResponseStatus::EventReceived, Some(rr))
                if rr.contains(ResponseRequired::MessageReceived) => {}
            (ResponseStatus::EventAcknowledge, Some(rr))
                if rr.contains(ResponseRequired::ResponseRequired) => {}
            (ResponseStatus::EventNoDisplay, _) => {
                bail!("Attempted to send a response for an event where one was not required, either due to it's status or the event's responseRequired field.")
            }
            (_, Some(rr)) if rr.contains(ResponseRequired::SpecificResponse) => {}
            _ => bail!("Attempted to send a response for an event where one was not required, either due to it's status or the event's responseRequired field."),
        };

        let resp = DrResponse {
            created_date_time: Some(time.into()),
            end_device_lfdi: device.lfdi,
            status: Some(status),
            subject: event.mrid,
            href: None,
            appliance_load_reduction: device.appliance_load_reduction,
            applied_target_reduction: device.applied_target_reduction,
            duty_cycle: device.duty_cycle,
            offset: device.offset.clone(),
            override_duration: device.override_duration,
            set_point: device.set_point.clone(),
        };
        self.put_post(
            event
                .reply_to()
                .context("Event does not contain a ReplyTo Field")?
                .parse()
                .context("Failed to parse ReplyTo Field")?,
            &resp,
            Method::POST,
            time,
        )
        .await
    }

    #[cfg(feature = "pricing")]
    pub async fn send_pricing_response(
        &self,
        lfdi: HexBinary160,
        event: &TimeTariffInterval,
        status: ResponseStatus,
        time: SEPTime,
    ) -> Result<SEPResponse> {
        // As per Pricing in Table 27
        match (status, event.response_required) {
            (ResponseStatus::EventReceived, Some(rr))
                if rr.contains(ResponseRequired::MessageReceived) => {}
            (ResponseStatus::EventStarted, Some(rr))
                if rr.contains(ResponseRequired::SpecificResponse) => {}
            (ResponseStatus::EventCompleted, Some(rr))
                if rr.contains(ResponseRequired::SpecificResponse) => {}
            (ResponseStatus::EventSuperseded, Some(rr))
                if rr.contains(ResponseRequired::SpecificResponse) => {}
            (ResponseStatus::EventAcknowledge, Some(rr))
                if rr.contains(ResponseRequired::ResponseRequired) => {}
            (ResponseStatus::EventAbortedServer, Some(rr))
                if rr.contains(ResponseRequired::SpecificResponse) => {}
            (ResponseStatus::EventAbortedProgram, Some(rr))
                if rr.contains(ResponseRequired::SpecificResponse) => {}
            (ResponseStatus::EventExpired, Some(rr))
                if rr.contains(ResponseRequired::SpecificResponse) => {}
            _ => bail!("Attempted to send a response for an event where one was not required, either due to it's status or the event's responseRequired field.")
        };
        let resp = PriceResponse {
            created_date_time: Some(time.into()),
            end_device_lfdi: lfdi,
            status: Some(status),
            subject: event.mrid,
            href: None,
        };
        self.put_post(
            event
                .reply_to()
                .context("Event does not contain a ReplyTo Field")?
                .parse()
                .context("Failed to parse ReplyTo Field")?,
            &resp,
            Method::POST,
            time,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sep2_common::packages::edev::EndDevice;

    // A test client which will never run its polls in the test time window due
    // to the large default interval. This allows testing without a server.
    fn test_client() -> Client {
        Client::new("http://127.0.0.1:0", None, None).unwrap()
    }

    async fn start_noop_poll(client: &Client, rate: u32) -> PollId {
        client
            .start_poll("/edev", Some(Uint32(rate)), |_: EndDevice| async {})
            .await
    }

    async fn job_state(client: &Client, id: PollId) -> Option<(Duration, Instant)> {
        client
            .polls
            .lock()
            .await
            .iter()
            .find(|poll| poll.id == id)
            .map(|poll| (poll.interval, poll.next))
    }

    #[tokio::test]
    async fn poll_ids_are_unique() {
        let client = test_client();
        let a = start_noop_poll(&client, 900).await;
        let b = start_noop_poll(&client, 900).await;
        let c = start_noop_poll(&client.clone(), 900).await;
        assert!(a != b && b != c && a != c);
        assert_eq!(client.polls.lock().await.len(), 3);
    }

    #[tokio::test]
    async fn cancel_poll_removes_only_target() {
        let client = test_client();
        let a = start_noop_poll(&client, 900).await;
        let b = start_noop_poll(&client, 900).await;

        client.cancel_poll(a).await;
        assert!(job_state(&client, a).await.is_none());
        assert!(job_state(&client, b).await.is_some());

        // Cancelling an already-cancelled poll is a no-op
        client.cancel_poll(a).await;
        assert_eq!(client.polls.lock().await.len(), 1);
    }

    #[tokio::test]
    async fn update_poll_rate_shorter_brings_next_forward() {
        let client = test_client();
        let id = start_noop_poll(&client, 900).await;
        let (_, old_next) = job_state(&client, id).await.unwrap();

        client.update_poll_rate(id, Uint32(10)).await;
        let (interval, next) = job_state(&client, id).await.unwrap();
        assert_eq!(interval, Duration::from_secs(10));
        assert!(next < old_next);
        assert!(next <= Instant::now() + Duration::from_secs(10));
    }

    #[tokio::test]
    async fn update_poll_rate_longer_keeps_next() {
        let client = test_client();
        let id = start_noop_poll(&client, 10).await;
        let (_, old_next) = job_state(&client, id).await.unwrap();

        client.update_poll_rate(id, Uint32(900)).await;
        let (interval, next) = job_state(&client, id).await.unwrap();
        assert_eq!(interval, Duration::from_secs(900));
        assert_eq!(next, old_next);
    }

    #[tokio::test]
    async fn update_poll_rate_keeps_heap_ordered() {
        let client = test_client();
        let a = start_noop_poll(&client, 900).await;
        let b = start_noop_poll(&client, 600).await;
        assert_eq!(client.polls.lock().await.peek().unwrap().id, b);

        client.update_poll_rate(a, Uint32(10)).await;
        assert_eq!(client.polls.lock().await.peek().unwrap().id, a);
    }
}
