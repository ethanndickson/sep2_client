use std::{collections::HashMap, sync::Arc};

use common::packages::{
    identification::ResponseStatus,
    objects::EventStatusType,
    traits::SEEvent,
    types::{Mridtype, OneHourRangeType, PrimacyType},
    xsd::EndDevice,
};
use log::error;
use rand::Rng;
use tokio::sync::RwLock;

use crate::{client::Client, time::current_time};

struct EventInstance<E: SEEvent> {
    start: i64,
    end: i64,
    primacy: PrimacyType,
    status: EventInstanceStatus,
    event: E,
}

enum EventInstanceStatus {
    Scheduled,
    Active,
    Cancelled,
    CancelledRandom,
    Superseded,
    // Add more as needed
    // Aborted,
    // Completed,
    // InProgress,
    // ScheduleSuperseded,
}

impl From<EventStatusType> for EventInstanceStatus {
    fn from(value: EventStatusType) -> Self {
        match value {
            EventStatusType::Scheduled => EventInstanceStatus::Scheduled,
            EventStatusType::Active => EventInstanceStatus::Active,
            EventStatusType::Cancelled => EventInstanceStatus::Cancelled,
            EventStatusType::CancelledRandom => EventInstanceStatus::CancelledRandom,
            EventStatusType::Superseded => EventInstanceStatus::Superseded,
        }
    }
}

// Compare EventInstances (Non-Active (Scheduled / Superseded)) by their effective start time
impl<E: SEEvent> PartialEq for EventInstance<E> {
    fn eq(&self, other: &Self) -> bool {
        self.start == other.start
    }
}

impl<E: SEEvent> Eq for EventInstance<E> {}

impl<E: SEEvent> PartialOrd for EventInstance<E> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.start.partial_cmp(&other.start)
    }
}

impl<E: SEEvent> Ord for EventInstance<E> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.start.cmp(&other.start)
    }
}

// A wrapper around an event instance to allow it to be ordered by event end time
struct ActiveEventInstance<E: SEEvent>(EventInstance<E>);

// Compare Active EventInstances by their effective end time
impl<E: SEEvent> PartialEq for ActiveEventInstance<E> {
    fn eq(&self, other: &Self) -> bool {
        self.0.end == other.0.end
    }
}

impl<E: SEEvent> Eq for ActiveEventInstance<E> {}

impl<E: SEEvent> PartialOrd for ActiveEventInstance<E> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.0.end.partial_cmp(&other.0.end)
    }
}

impl<E: SEEvent> Ord for ActiveEventInstance<E> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.end.cmp(&other.0.end)
    }
}

impl<E: SEEvent> EventInstance<E> {
    pub fn new(event: E, primacy: PrimacyType) -> Self {
        let start: i64 = event.interval().start.get();
        let end: i64 = start + i64::from(*event.interval().duration);
        EventInstance {
            status: event.event_status().current_status.clone().into(),
            event,
            primacy,
            start,
            end,
        }
    }

    pub fn new_rand(
        primacy: PrimacyType,
        rand_duration: Option<OneHourRangeType>,
        rand_start: Option<OneHourRangeType>,
        event: E,
    ) -> Self {
        let start: i64 = event.interval().start.get() + randomize(rand_duration);
        let end: i64 = start + i64::from(*event.interval().duration) + randomize(rand_start);
        EventInstance {
            status: event.event_status().current_status.clone().into(),
            event,
            primacy,
            start,
            end,
        }
    }

    pub fn supersedes(&self, other: Self) -> bool {
        self.primacy == other.primacy && self.event.creation_time() > other.event.creation_time()
            || self.primacy < other.primacy
    }

    pub fn update_primacy(&mut self, primacy: PrimacyType) {
        self.primacy = primacy;
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

pub struct Schedule<E: SEEvent> {
    client: Client,
    // Send + Sync end device, as the EndDevice resource may be updated
    device: Arc<RwLock<EndDevice>>,
    // Lookup by MRID
    events: HashMap<Mridtype, EventInstance<E>>,
    // scheduled: BinaryHeap<&'a mut EventInstance<E>>,
    // active: BinaryHeap<&'a mut ActiveEventInstance<E>>,
    // superseded: BinaryHeap<&'a mut EventInstance<E>>,
}

impl<E: SEEvent> Schedule<E> {
    /// Create a schedule for the given client & it's EndDevice representation
    pub fn new(client: Client, device: Arc<RwLock<EndDevice>>) -> Self {
        Schedule {
            client,
            device,
            events: HashMap::new(),
        }
    }

    pub async fn schedule_event(&mut self, event: E, primacy: PrimacyType) {
        self.schedule_rand_event(event, primacy, None, None).await
    }

    pub async fn schedule_rand_event(
        &mut self,
        event: E,
        primacy: PrimacyType,
        // Ideally we would have a seperate function for RandomizableEvents,
        // however there is noway to downcast a generic ST: SERandomizableEvent to a T: SEEvent,
        // even if SERandomizableEvent : SEEvent.
        // Instead we take the extra fields from that RandomizableEvent
        randomize_duration: Option<OneHourRangeType>,
        randomize_start: Option<OneHourRangeType>,
    ) {
        let mrid = event.mrid().clone();
        let ev = self.events.get_mut(&mrid);

        if let Some(ev) = ev {
            ev.primacy = primacy;
        } else {
            // Inform server event was scheduled
            self.send_schedule_resp(&event, ResponseStatus::EventReceived)
                .await;
            // Calculate start & end times
            let ei = EventInstance::new_rand(primacy, randomize_duration, randomize_start, event);
            // The event may have expired already
            if ei.end <= current_time().get() {
                self.send_schedule_resp(&ei.event, ResponseStatus::EventExpired)
                    .await;
                return;
            }
            // Add it to our schedule
            self.events.insert(mrid, ei);
        }

        // TODO: Handle event status
    }

    // Schedule a new event, logging if it fails
    async fn send_schedule_resp(&mut self, event: &E, status: ResponseStatus) {
        if let Some(lfdi) = { self.device.read().await.lfdi } {
            let _ = self
                .client
                .send_der_response(lfdi, event, status)
                .await
                .map_err(|e| error!("DER response POST attempt failed with reason: {}", e));
        } else {
            error!("Attempted to send DER response for EndDevice that does not have an LFDI")
        }
    }
}
