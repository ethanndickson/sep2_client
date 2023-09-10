use std::{
    collections::HashMap,
    future::Future,
    sync::Arc,
    time::{Duration, SystemTime},
};

use crate::{client::Client, time::current_time};
use common::packages::{
    identification::ResponseStatus,
    objects::{Dercontrol, EndDeviceControl, EventStatusType, TextMessage, TimeTariffInterval},
    traits::SEEvent,
    types::{Mridtype, OneHourRangeType, PrimacyType},
    xsd::{EndDevice, FlowReservationResponse},
};
use rand::Rng;
use tokio::sync::oneshot::{self, Receiver, Sender};
use tokio::sync::RwLock;

pub struct EventInstance<E: SEEvent> {
    pub start: i64,
    pub end: i64,
    pub primacy: PrimacyType,
    pub status: EventStatusType,
    pub event: E,
    oneshot: (Sender<EventUpdate>, Option<Receiver<EventUpdate>>),
}

/// Started denotes the event has started, for any reason.
/// Finished denotes the event has ended, for any reason.
pub enum EventUpdate {
    Started,
    Complete,
    Cancelled,
    Superseded,
    InternalError,
}

impl From<EventUpdate> for ResponseStatus {
    fn from(value: EventUpdate) -> Self {
        todo!()
    }
}

impl EventInstance<Dercontrol> {
    fn der_supersedes(&self, other: &Self) -> bool {
        // TODO: Confirm this is correct
        if self.primacy == other.primacy && self.event.creation_time() > other.event.creation_time()
            || self.primacy < other.primacy
        {
            self.event
                .der_control_base
                .same_target(&other.event.der_control_base)
        } else {
            false
        }
    }
}

impl<E: SEEvent> EventInstance<E> {
    fn new(event: E, primacy: PrimacyType) -> Self {
        let start: i64 = event.interval().start.get();
        let end: i64 = start + i64::from(event.interval().duration.get());
        let (send, recv) = oneshot::channel();
        EventInstance {
            status: event.event_status().current_status,
            event,
            primacy,
            start,
            end,
            oneshot: (send, Some(recv)),
        }
    }

    fn new_rand(
        primacy: PrimacyType,
        rand_duration: Option<OneHourRangeType>,
        rand_start: Option<OneHourRangeType>,
        event: E,
    ) -> Self {
        let start: i64 = event.interval().start.get() + randomize(rand_duration);
        let end: i64 = start + i64::from(event.interval().duration.get()) + randomize(rand_start);
        let (send, recv) = oneshot::channel();
        EventInstance {
            status: event.event_status().current_status,
            event,
            primacy,
            start,
            end,
            oneshot: (send, Some(recv)),
        }
    }

    fn supersedes(&self, other: &Self) -> bool {
        self.primacy == other.primacy && self.event.creation_time() > other.event.creation_time()
            || self.primacy < other.primacy
    }
}

fn randomize(bound: Option<OneHourRangeType>) -> i64 {
    bound.map_or(0, |val| {
        let val = val.get();
        let mut rng = rand::thread_rng();
        let sign = val.signum() as i64;
        rng.gen_range(0..=val.abs().into()) * sign
    })
}

pub struct Schedule<E, F, Res>
where
    E: SEEvent,
    F: FnMut(Arc<RwLock<EventInstance<E>>>, EventUpdate) -> Res + Send + Sync + Clone + 'static,
    Res: Future<Output = ResponseStatus> + Send,
{
    client: Client,
    // Send + Sync end device, as the EndDevice resource may be updated
    device: Arc<RwLock<EndDevice>>,
    // Lookup by MRID
    events: HashMap<Mridtype, Arc<RwLock<EventInstance<E>>>>,
    // User-defined callback for informing user of event state transitions
    callback: F,
}

impl<E, F, Res> Schedule<E, F, Res>
where
    E: SEEvent,
    F: FnMut(Arc<RwLock<EventInstance<E>>>, EventUpdate) -> Res + Send + Sync + Clone + 'static,
    Res: Future<Output = ResponseStatus> + Send,
{
    /// Create a schedule for the given client & it's EndDevice representation
    pub fn new(client: Client, device: Arc<RwLock<EndDevice>>, callback: F) -> Self {
        Schedule {
            client,
            device,
            events: HashMap::new(),
            callback,
        }
    }
}

// Distributed Energy Resources Function Set
impl<F, Res> Schedule<Dercontrol, F, Res>
where
    F: FnMut(Arc<RwLock<EventInstance<Dercontrol>>>, EventUpdate) -> Res
        + Send
        + Sync
        + Clone
        + 'static,
    Res: Future<Output = ResponseStatus> + Send,
{
    pub async fn schedule_event(&mut self, event: Dercontrol, primacy: PrimacyType) {
        let mrid = event.mrid;
        let ev = self.events.get_mut(&mrid);
        let incoming_status = event.event_status.current_status;

        // If the event already exists in the schedule
        let ei = if let Some(ei) = ev {
            // "Editing events shall NOT be allowed, except for updating status"
            ei.clone()
        } else {
            // Inform server event was scheduled
            self.client
                .send_der_response(
                    self.device.read().await.lfdi,
                    &event,
                    ResponseStatus::EventReceived,
                )
                .await;
            // Calculate start & end times
            let ei = EventInstance::new_rand(
                primacy,
                event.randomize_duration,
                event.randomize_start,
                event,
            );
            // The event may have expired already
            if ei.end <= current_time().get() {
                // Do not add event to schedule
                self.client
                    .send_der_response(
                        self.device.read().await.lfdi,
                        &ei.event,
                        ResponseStatus::EventExpired,
                    )
                    .await;
                return;
            }
            // Add it to our schedule
            let ei = Arc::new(RwLock::new(ei));
            self.events.insert(mrid, ei.clone());
            ei
        };

        // Handle status transitions
        let current_status = ei.read().await.status;
        match (current_status, incoming_status) {
            // (Cancelled | CancelledRandom | Superseded) -> Any
            (
                EventStatusType::Cancelled
                | EventStatusType::CancelledRandom
                | EventStatusType::Superseded,
                _,
            ) => (),
            // Scheduled -> (Cancelled || CancelledRandom)
            (
                EventStatusType::Scheduled,
                s @ (EventStatusType::Cancelled | EventStatusType::CancelledRandom),
            ) => {
                // Respond EventCancelled
                self.client
                    .send_der_response(
                        self.device.read().await.lfdi,
                        &ei.read().await.event,
                        s.into(),
                    )
                    .await;
                // TODO: What do we need to keep the cancelled event for?
                self.events.remove(&mrid);
            }
            // Active -> Active - Do nothing
            (EventStatusType::Active, EventStatusType::Active) => (),
            // Scheduled -> Active - Start event early
            (EventStatusType::Scheduled, EventStatusType::Active) => todo!(),
            // Active -> Superseded
            (EventStatusType::Active, EventStatusType::Superseded) => todo!(),
            // Scheduled -> Superseded
            (EventStatusType::Scheduled, EventStatusType::Superseded) => todo!(),
            // Active -> Scheduled
            (EventStatusType::Active, EventStatusType::Scheduled) => todo!(),
            // Active -> Cancelled
            (EventStatusType::Active, EventStatusType::Cancelled) => todo!(),
            // Active -> CancelledRandom
            (EventStatusType::Active, EventStatusType::CancelledRandom) => todo!(),
            // Scheduled -> Scheduled
            (EventStatusType::Scheduled, EventStatusType::Scheduled) => todo!(),
        }
    }

    // Internal Event State transitions forbid this function from running multiple times for a single event
    async fn start_event(&mut self, mrid: &Mridtype) {
        // Guaranteed to exist, avoid double mut borrow
        let target_ei = self.events.remove(mrid).unwrap();
        let mut superseded: Vec<Mridtype> = vec![];
        // Mark required events as superseded
        for (mrid, ei) in &mut self.events {
            let ei = &mut *ei.write().await;
            if (target_ei.read().await).der_supersedes(ei) {
                // Notify client active event has ended
                if matches!(ei.status, EventStatusType::Active) {
                    // TODO: Callback w/ EVENT END
                }
                ei.status = EventStatusType::Superseded;
                superseded.push(*mrid);
            }
        }
        // Notify server
        for mrid in superseded {
            let event = &self.events.get(&mrid).unwrap().read().await.event;
            self.client
                .send_der_response(
                    self.device.read().await.lfdi,
                    event,
                    ResponseStatus::EventSuperseded,
                )
                .await;
        }
        // Update event status
        target_ei.write().await.status = EventStatusType::Active;
        // Setup task data
        let event_duration = (target_ei.read().await.end
            - (SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("Time went backwards")
                .as_secs() as i64))
            .max(0) as u64;
        let mut callback = self.callback.clone();
        let target_event_c = target_ei.clone();
        let client = self.client.clone();
        let lfdi = self.device.read().await.lfdi;
        let rx = target_ei.write().await.oneshot.1.take().unwrap();
        // Create task that waits until the event is finished, with the ability to end it early
        tokio::spawn(async move {
            let ei = target_event_c.clone();
            let resp = tokio::select! {
                    // Wait until event end, then inform client.
                    _ = tokio::time::sleep(Duration::from_secs(event_duration)) => callback(ei, EventUpdate::Complete),
                    // Unless we're told to cancel the event
                    status = rx => {
                        if let Ok(status) = status {
                            callback(ei, status)
                        } else {
                            // TODO: Can this be replaced with an unwrap? Can the Sender be dropped before the receiver?
                            callback(ei, EventUpdate::InternalError)
                        }

                    }
                }.await;
            // Return the user-defined response
            client
                .send_der_response(lfdi, &target_event_c.read().await.event, resp)
                .await;
        });

        // Inform client of event start
        (self.callback)(target_ei.clone(), EventUpdate::Started);
        // Store event
        self.events.insert(*mrid, target_ei);
    }

    // Events may be cancelled before they complete.
    async fn cancel_event(&mut self, mrid: &Mridtype) {
        todo!()
    }
}

// Demand Response Load Control Function Set
impl<F, Res> Schedule<EndDeviceControl, F, Res>
where
    F: FnMut(Arc<RwLock<EventInstance<EndDeviceControl>>>, EventUpdate) -> Res
        + Send
        + Sync
        + Clone
        + 'static,
    Res: Future<Output = ResponseStatus> + Send,
{
}

// Messaging Function Set
impl<F, Res> Schedule<TextMessage, F, Res>
where
    F: FnMut(Arc<RwLock<EventInstance<TextMessage>>>, EventUpdate) -> Res
        + Send
        + Sync
        + Clone
        + 'static,
    Res: Future<Output = ResponseStatus> + Send,
{
}

// Flow Reservation Function Set
impl<F, Res> Schedule<FlowReservationResponse, F, Res>
where
    F: FnMut(Arc<RwLock<EventInstance<FlowReservationResponse>>>, EventUpdate) -> Res
        + Send
        + Sync
        + Clone
        + 'static,
    Res: Future<Output = ResponseStatus> + Send,
{
}

// Pricing Function Set
impl<F, Res> Schedule<TimeTariffInterval, F, Res>
where
    F: FnMut(Arc<RwLock<EventInstance<TimeTariffInterval>>>, EventUpdate) -> Res
        + Send
        + Sync
        + Clone
        + 'static,
    Res: Future<Output = ResponseStatus> + Send,
{
}
