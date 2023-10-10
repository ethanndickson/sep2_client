use std::{
    collections::{hash_map, HashMap},
    sync::Arc,
};

use crate::{
    client::Client,
    time::{current_time, SLEEP_TICKRATE},
};
use async_trait::async_trait;
use rand::Rng;
use sep2_common::packages::{
    edev::EndDevice,
    identification::ResponseStatus,
    objects::EventStatusType,
    types::{MRIDType, OneHourRangeType, PrimacyType},
};
use sep2_common::traits::SEEvent;
use tokio::sync::RwLock;

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
    // TODO: ScheduledSuperseded & Unsuperseding unstarted events
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
        }
    }

    pub(crate) fn supersedes(&self, other: &Self) -> bool {
        self.primacy == other.primacy && self.event.creation_time() > other.event.creation_time()
            || self.primacy < other.primacy
    }

    pub(crate) fn update_status(&mut self, status: EIStatus) {
        self.status = status;
        self.last_updated = current_time().get();
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
    async fn event_update(&self, event: &EventInstance<E>, status: EIStatus) -> ResponseStatus;
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
    pub(crate) fn iter(&self) -> hash_map::Iter<'_, MRIDType, EventInstance<E>> {
        self.map.iter()
    }

    pub(crate) fn insert(&mut self, mrid: &MRIDType, ei: EventInstance<E>) {
        if ei.status() == EIStatus::Scheduled {
            match self.next_start {
                Some(next_start) if ei.end < next_start.0 => {
                    self.next_start = Some((ei.start, *mrid))
                }
                None => self.next_start = Some((ei.start, *mrid)),
                _ => (),
            }
        }
        if ei.status() == EIStatus::Active {
            match self.next_end {
                Some(next_end) if ei.end < next_end.0 => self.next_end = Some((ei.end, *mrid)),
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

    pub(crate) fn update_events(&mut self, events: &[MRIDType], status: EIStatus) {
        for o_mrid in events {
            let other = self.map.get_mut(o_mrid).unwrap();
            other.update_status(status);
        }
        self.update_nexts();
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
pub struct Schedule<E, H>
where
    E: SEEvent,
    H: EventHandler<E>,
{
    pub(crate) client: Client,
    // Send + Sync end device, as the EndDevice resource may be updated
    pub(crate) device: Arc<RwLock<EndDevice>>,
    // All Events added to this schedule, indexed by mRID
    pub(crate) events: Arc<RwLock<Events<E>>>,
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
    pub(crate) async fn clean_events(self) {
        let week = 60 * 60 * 24 * 30;
        let mut last = current_time().get();
        let mut next = current_time().get() + week;
        loop {
            crate::time::sleep_until(next, SLEEP_TICKRATE).await;
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
}
