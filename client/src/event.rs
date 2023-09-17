use std::{collections::HashMap, sync::Arc};

use crate::client::Client;
use async_trait::async_trait;
use common::packages::{
    identification::ResponseStatus,
    objects::{EndDeviceControl, EventStatusType, TextMessage, TimeTariffInterval},
    types::{MRIDType, OneHourRangeType, PrimacyType},
    xsd::{EndDevice, FlowReservationResponse},
};
use common::traits::SEEvent;
use rand::Rng;
use tokio::sync::RwLock;

/// A wrapper around an [`SEEvent`] resource.
pub struct EventInstance<E: SEEvent> {
    pub(crate) start: i64,
    pub(crate) end: i64,
    pub(crate) primacy: PrimacyType,
    /// The current status of the Event,
    pub(crate) status: EIStatus,
    pub(crate) event: E,
}

/// The current state of an [`EventInstance`] in the schedule.
/// Can be created from a [`EventStatusType`] for the purpose of reading [`SEEvent`] resources.
/// Can be converted to a [`ResponseStatus`] for the purpose of creating [`SEResponse`] resources.
///
/// [`SEResponse`]: common::traits::SEResponse
#[derive(PartialEq, Eq, Debug, Clone, Copy)]
#[repr(u8)]
pub enum EIStatus {
    Scheduled,
    Active,
    Cancelled,
    Complete,
    CancelledRandom,
    Superseded,
    // TODO: ScheduledSuperseded & Unsuperseding unstarted events
}

impl From<EIStatus> for ResponseStatus {
    fn from(value: EIStatus) -> Self {
        match value {
            EIStatus::Scheduled => Self::EventReceived, // TODO:  Maybe
            EIStatus::Active => Self::EventStarted,
            EIStatus::Cancelled => Self::EventCancelled,
            EIStatus::CancelledRandom => Self::EventCancelled,
            EIStatus::Superseded => Self::EventSuperseded,
            EIStatus::Complete => Self::EventCompleted,
        }
    }
}

impl From<EventStatusType> for EIStatus {
    fn from(value: EventStatusType) -> Self {
        match value {
            EventStatusType::Scheduled => Self::Scheduled,
            EventStatusType::Active => Self::Active,
            EventStatusType::Cancelled => Self::Cancelled,
            EventStatusType::CancelledRandom => Self::CancelledRandom,
            EventStatusType::Superseded => Self::Superseded,
        }
    }
}

impl<E: SEEvent> EventInstance<E> {
    pub(crate) fn new(event: E, primacy: PrimacyType) -> Self {
        let start: i64 = event.interval().start.get();
        let end: i64 = start + i64::from(event.interval().duration.get());
        EventInstance {
            status: event.event_status().current_status.into(),
            event,
            primacy,
            start,
            end,
        }
    }

    pub(crate) fn new_rand(
        primacy: PrimacyType,
        rand_duration: Option<OneHourRangeType>,
        rand_start: Option<OneHourRangeType>,
        event: E,
    ) -> Self {
        let start: i64 = event.interval().start.get() + randomize(rand_duration);
        let end: i64 = start + i64::from(event.interval().duration.get()) + randomize(rand_start);
        EventInstance {
            status: event.event_status().current_status.into(),
            event,
            primacy,
            start,
            end,
        }
    }

    pub(crate) fn supersedes(&self, other: &Self) -> bool {
        self.primacy == other.primacy && self.event.creation_time() > other.event.creation_time()
            || self.primacy < other.primacy
    }

    pub fn status(&self) -> EIStatus {
        self.status
    }

    pub fn event(&self) -> &E {
        &self.event
    }

    pub fn primacy(&self) -> &PrimacyType {
        &self.primacy
    }

    pub fn start_time(&self) -> i64 {
        self.start
    }

    pub fn end_time(&self) -> i64 {
        self.start
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

#[async_trait]
pub trait EventHandler<E: SEEvent>: Send + Sync + 'static {
    /// Called whenever the state of an event is updated such that a response to the server is required.
    /// Type is bound by an [`SEEvent`] pertaining to a specific function set.
    ///
    /// Allows the client to apply the event at the device-level, and determine the correct response code.
    ///
    /// When determining the ResponseStatus to return, refer to Table 27 of IEEE 2030.5-2018
    async fn event_update(
        &self,
        event: Arc<RwLock<EventInstance<E>>>,
        status: EIStatus,
    ) -> ResponseStatus;
}

type EventsMap<E> = Arc<RwLock<HashMap<MRIDType, Arc<RwLock<EventInstance<E>>>>>>;

/// Schedule for a given function set, and a specific server.
///
/// Schedules are bound by the [`SEEvent`] pertaining to a specific function set,
/// and an [`EventHandler`] that is passed event updates, such that clients can make device changes,
/// and dictate the response sent to the server.
///
/// Multi-server interactions are handled gracefully as the `replyTo` field on Events contains the hostname of the server.
pub struct Schedule<E, H>
where
    E: SEEvent,
    H: EventHandler<E>,
{
    pub(crate) client: Client,
    // Send + Sync end device, as the EndDevice resource may be updated
    pub(crate) device: Arc<RwLock<EndDevice>>,
    // All Events added to this schedule, indexed by mRID
    pub(crate) events: EventsMap<E>,
    // Callback provider for informing client of event state transitions
    pub(crate) handler: Arc<H>,
}

// Manual clone implementation since H doesn't need to be clone
impl<E, H> Clone for Schedule<E, H>
where
    E: SEEvent,
    H: EventHandler<E>,
{
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            device: self.device.clone(),
            events: self.events.clone(),
            handler: self.handler.clone(),
        }
    }
}

impl<E, H> Schedule<E, H>
where
    E: SEEvent,
    H: EventHandler<E>,
{
    /// Create a schedule for the given [`Client`] & it's [`EndDevice`] representation.
    ///
    /// Any instance of [`Client`] can be used, as responses are made in accordance to the hostnames within the provided events.
    pub fn new(client: Client, device: Arc<RwLock<EndDevice>>, handler: H) -> Self {
        Schedule {
            client,
            device,
            events: Arc::new(RwLock::new(HashMap::new())),
            handler: Arc::new(handler),
        }
    }
}

// Demand Response Load Control Function Set
impl<H: EventHandler<EndDeviceControl>> Schedule<EndDeviceControl, H> {}

// Messaging Function Set
impl<H: EventHandler<TextMessage>> Schedule<TextMessage, H> {}

// Flow Reservation Function Set
impl<H: EventHandler<FlowReservationResponse>> Schedule<FlowReservationResponse, H> {}

// Pricing Function Set
impl<H: EventHandler<TimeTariffInterval>> Schedule<TimeTariffInterval, H> {}
