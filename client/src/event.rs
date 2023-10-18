//! Common Application Functionality: Events
//!
//!
//! Since we are required to compare real-world timestamps
//! to determine when events start and end, we cannot use the [`Instant`] type.
//! Instead, we compare an `i64` of the number of seconds since the Unix Epoch.

use std::{
    collections::{hash_map, HashMap},
    sync::Arc,
    time::Duration,
};

use crate::{client::Client, edev::SEDevice, time::current_time};
use async_trait::async_trait;
use rand::Rng;
use sep2_common::packages::{
    identification::ResponseStatus,
    objects::EventStatusType,
    types::{MRIDType, OneHourRangeType, PrimacyType},
};
use sep2_common::traits::SEEvent;
use tokio::sync::{
    broadcast::{Receiver, Sender},
    RwLock,
};

/// A wrapper around an [`SEEvent`] resource.
pub struct EventInstance<E: SEEvent> {
    // Event start time, after randomisation
    start: i64,
    // Event end time, after randomisation
    end: i64,
    // Event primacy
    primacy: PrimacyType,
    // The SEEvent instance
    event: E,
    // The current status of the Event,
    status: EIStatus,
    // When the event status was last updated
    last_updated: i64,
    // The event(s) this supersedes, if any
    superseded_by: Vec<MRIDType>,
}

/// The current state of an [`EventInstance`] in the schedule.
/// Can be created from a [`EventStatusType`] for the purpose of reading [`SEEvent`] resources.
/// Can be converted to a [`ResponseStatus`] for the purpose of creating [`SEResponse`] resources.
///
/// [`SEResponse`]: sep2_common::traits::SEResponse
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

impl EIStatus {
    /// Given the status of an EventInstance, convert it to a ResponseStatus, as per the DER FS
    pub fn into_der_response(self) -> ResponseStatus {
        match self {
            EIStatus::Scheduled => ResponseStatus::EventReceived,
            EIStatus::Active => ResponseStatus::EventStarted,
            EIStatus::Cancelled => ResponseStatus::EventCancelled,
            EIStatus::Complete => ResponseStatus::EventCompleted,
            EIStatus::CancelledRandom => ResponseStatus::EventCancelled,
            EIStatus::Superseded => ResponseStatus::EventSuperseded,
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
            last_updated: current_time().get(),
            superseded_by: vec![],
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
            last_updated: current_time().get(),
            superseded_by: vec![],
        }
    }

    // Determine if the this event supersedes the other
    pub(crate) fn does_supersede(&self, other: &Self) -> bool {
        // If there is an overlap
        self.start_time() <= other.end_time()
            && self.end_time() >= other.start_time()
            // If other has lesser primacy
            && (self.primacy < other.primacy
                // Or same primacy, and this one is newer
                || self.primacy == other.primacy
                    && self.event.creation_time() > other.event.creation_time())
    }

    /// Given two events, determine which is superseded, update internal states, and return the superseded event
    pub(crate) fn mark_supersede<'a>(
        a: (&'a mut EventInstance<E>, &'a MRIDType),
        b: (&'a mut EventInstance<E>, &'a MRIDType),
    ) -> Option<&'a mut EventInstance<E>> {
        let out = if a.0.does_supersede(b.0) {
            Some((a, b))
        } else if b.0.does_supersede(a.0) {
            Some((b, a))
        } else {
            None
        };

        out.map(|(superseding, superseded)| {
            superseded.0.superseded_by(superseding.1);
            superseded.0.update_status(EIStatus::Superseded);
            superseding.0
        })
    }

    pub(crate) fn update_status(&mut self, status: EIStatus) {
        self.status = status;
        self.last_updated = current_time().get();
    }

    pub(crate) fn superseded_by(&mut self, other: &MRIDType) {
        self.superseded_by.push(*other);
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
        self.end
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

// This trait uses extra heap allocations while we await stable RPITIT (and eventually async fn with a send bound future)
#[async_trait]
pub trait EventHandler<E: SEEvent>: Send + Sync + 'static {
    /// Called whenever the state of an event is updated such that a response to the server is required.
    /// Type is bound by an [`SEEvent`] pertaining to a specific function set.
    ///
    /// Allows the client to apply the event at the device-level, and determine the correct response code.
    ///
    /// When determining the ResponseStatus to return, refer to Table 27 of IEEE 2030.5-2018
    async fn event_update(&self, event: &EventInstance<E>) -> ResponseStatus;
}

type EventsMap<E> = HashMap<MRIDType, EventInstance<E>>;

/// Wrapper around a map of MRIDs to EventInstances, to maintain `next_start` and `next_end` validity
/// All functions
pub(crate) struct Events<E>
where
    E: SEEvent,
{
    map: EventsMap<E>,
    next_start: Option<(i64, MRIDType)>,
    next_end: Option<(i64, MRIDType)>,
}

impl<E> Events<E>
where
    E: SEEvent,
{
    pub(crate) fn new() -> Self {
        Events {
            map: HashMap::new(),
            next_start: None,
            next_end: None,
        }
    }

    #[inline(always)]
    pub(crate) fn next_start(&self) -> Option<(i64, MRIDType)> {
        self.next_start
    }

    #[inline(always)]
    pub(crate) fn next_end(&self) -> Option<(i64, MRIDType)> {
        self.next_end
    }

    #[inline(always)]
    pub(crate) fn iter_mut(&mut self) -> hash_map::IterMut<'_, MRIDType, EventInstance<E>> {
        self.map.iter_mut()
    }

    pub(crate) fn insert(&mut self, mrid: &MRIDType, ei: EventInstance<E>) {
        if ei.status() == EIStatus::Scheduled {
            match self.next_start {
                Some((start, _)) if ei.start < start => self.next_start = Some((ei.start, *mrid)),
                None => self.next_start = Some((ei.start, *mrid)),
                _ => (),
            }
        }
        if ei.status() == EIStatus::Active {
            match self.next_end {
                Some((end, _)) if ei.end < end => self.next_end = Some((ei.end, *mrid)),
                None => self.next_end = Some((ei.end, *mrid)),
                _ => (),
            }
        }
        let _ = self.map.insert(*mrid, ei);
    }

    #[inline(always)]
    pub(crate) fn get(&self, mrid: &MRIDType) -> Option<&EventInstance<E>> {
        self.map.get(mrid)
    }

    #[inline(always)]
    pub(crate) fn contains(&self, mrid: &MRIDType) -> bool {
        self.map.contains_key(mrid)
    }

    pub(crate) fn update_event(&mut self, event: &MRIDType, status: EIStatus) {
        let event = self.map.get_mut(event).unwrap();
        event.update_status(status);
        self.update_nexts();
    }

    /// Given an event, and a reason for it's cancellation:
    /// - unsupersede (reschedule) unstarted events that are no longer superseded by any other events
    /// - mark the given event as cancelled, and re-evaulate the upcoming events
    pub(crate) fn cancel_event(&mut self, event: &MRIDType, reason: EIStatus) {
        self.map
            .iter_mut()
            .filter(|(_, ei)| ei.status() == EIStatus::Superseded)
            .for_each(|(_, ei)| {
                // Current event is no longer superseded by the given event
                ei.superseded_by.retain(|f| f != event);
                // If the event is no longer superseded by anything, and hasn't started yet, reschedule it
                if ei.superseded_by.is_empty() && ei.start > current_time().get() {
                    ei.update_status(EIStatus::Scheduled)
                }
            });
        // Mark the event as cancelled internally, re-evaluate the next event to start
        self.update_event(event, reason);
    }

    /// Reevaluates `next_start` and `next_end`
    pub(crate) fn update_nexts(&mut self) {
        self.next_start = self
            .map
            .iter()
            .filter(|(_, ei)| ei.status() == EIStatus::Scheduled)
            .min_by_key(|(_, ei)| ei.start)
            .map(|(mrid, ei)| (ei.start, *mrid));
        self.next_end = self
            .map
            .iter()
            .filter(|(_, ei)| ei.status() == EIStatus::Active)
            .min_by_key(|(_, ei)| ei.end)
            .map(|(mrid, ei)| (ei.end, *mrid));
    }
}

// This trait uses extra heap allocations while we await stable RPITIT (and eventually async fn with a send bound future)
#[async_trait]
pub trait Scheduler<E: SEEvent, H: EventHandler<E>> {
    fn new(
        client: Client,
        device: Arc<RwLock<SEDevice>>,
        handler: Arc<H>,
        tickrate: Duration,
    ) -> Self;
    async fn add_event(&mut self, event: E, primacy: PrimacyType);
}

/// Schedule for a given function set, and a specific server.
///
/// Schedules are bound by the [`SEEvent`] pertaining to a specific function set,
/// and an [`EventHandler`] that is passed event updates, such that clients can make device changes,
/// and dictate the response sent to the server.
///
/// Multi-server interactions are handled gracefully as the `replyTo` field on Events contains the hostname of the server.
///
/// A [`Schedule`] instance is a handle to a handful of shared state.
/// Cloning this struct is relatively cheap, and will involve incrementing all internal atomic reference counts.
///
/// However, schedules currently do not clean up their background tasks when all are dropped,
/// thus [`Schedule::shutdown`] will perform a graceful shutdown.
pub struct Schedule<E, H>
where
    E: SEEvent,
    H: EventHandler<E>,
{
    pub(crate) client: Client,
    // Send + Sync end device, as the EndDevice resource may be updated
    pub(crate) device: Arc<RwLock<SEDevice>>,
    // All Events added to this schedule, indexed by mRID
    pub(crate) events: Arc<RwLock<Events<E>>>,
    // Callback provider for informing client of event state transitions
    pub(crate) handler: Arc<H>,
    // Broadcast to tasks to shut them down
    pub(crate) bc_sd: Sender<()>,
    // How often schedule background tasks should wake to check for an event end or event start.
    // Our background tasks sleep intermittently as IEEE 2030.5 client devices may be sleepy.
    // Thread / Tokio sleeps do not make progress while the device itself is slept.
    pub(crate) tickrate: Duration,
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
            bc_sd: self.bc_sd.clone(),
            tickrate: self.tickrate.clone(),
        }
    }
}

impl<E, H> Schedule<E, H>
where
    E: SEEvent,
    H: EventHandler<E>,
{
    pub(crate) async fn clean_events(self, mut rx: Receiver<()>) {
        let week = 60 * 60 * 24 * 30;
        let mut last = current_time().get();
        let mut next = current_time().get() + week;
        loop {
            tokio::select! {
                _ = crate::time::sleep_until(next,self.tickrate) => (),
                _ = rx.recv() => {
                    log::info!("DERControlSchedule: Shutting down clean event task...");
                    break
                },
            }
            self.events.write().await.map.retain(|_, ei| {
                // Retain if:
                // Event is active or scheduled
                // or
                // Event has been updated since last prune
                matches!(ei.status, EIStatus::Active | EIStatus::Scheduled)
                    || ei.last_updated > last
            });
            last = current_time().get();
            next = last + week;
        }
    }

    pub fn shutdown(&mut self) {
        let _ = self.bc_sd.send(());
    }
}
