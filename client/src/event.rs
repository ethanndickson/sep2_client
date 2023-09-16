use std::{collections::HashMap, future::Future, sync::Arc};

use crate::client::Client;
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
/// [`SEResponse`]: common::packages::traits::SEResponse
#[derive(PartialEq, Eq, Debug, Clone, Copy)]
#[repr(u8)]
pub enum EIStatus {
    Scheduled,
    Active,
    Cancelled,
    Complete,
    CancelledRandom,
    Superseded,
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
    F: FnMut(Arc<RwLock<EventInstance<E>>>, EIStatus) -> Res + Send + Sync + Clone + 'static,
    Res: Future<Output = ResponseStatus> + Send,
{
    pub(crate) client: Client,
    // Send + Sync end device, as the EndDevice resource may be updated
    pub(crate) device: Arc<RwLock<EndDevice>>,
    // Lookup by MRID
    pub(crate) events: HashMap<MRIDType, Arc<RwLock<EventInstance<E>>>>,
    // User-defined callback for informing user of event state transitions
    pub(crate) callback: F,
}

impl<E, F, Res> Schedule<E, F, Res>
where
    E: SEEvent,
    F: FnMut(Arc<RwLock<EventInstance<E>>>, EIStatus) -> Res + Send + Sync + Clone + 'static,
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

// Demand Response Load Control Function Set
impl<F, Res> Schedule<EndDeviceControl, F, Res>
where
    F: FnMut(Arc<RwLock<EventInstance<EndDeviceControl>>>, EIStatus) -> Res
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
    F: FnMut(Arc<RwLock<EventInstance<TextMessage>>>, EIStatus) -> Res
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
    F: FnMut(Arc<RwLock<EventInstance<FlowReservationResponse>>>, EIStatus) -> Res
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
    F: FnMut(Arc<RwLock<EventInstance<TimeTariffInterval>>>, EIStatus) -> Res
        + Send
        + Sync
        + Clone
        + 'static,
    Res: Future<Output = ResponseStatus> + Send,
{
}
