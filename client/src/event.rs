use std::{collections::HashMap, sync::Arc};

use common::packages::{
    objects::PrimacyType,
    traits::{SEEvent, SERandomizableEvent},
    xsd::{EndDevice, Mridtype, OneHourRangeType},
};
use rand::Rng;
use tokio::sync::RwLock;

use crate::client::Client;

struct EventInstance<E: SEEvent> {
    event: E,
    start: i64,
    end: i64,
    primacy: PrimacyType,
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
        let start: i64 = *event.interval().start;
        let end: i64 = start + i64::from(*event.interval().duration);
        EventInstance {
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
        let start: i64 = *event.interval().start + randomize(rand_duration);
        let end: i64 = start + i64::from(*event.interval().duration) + randomize(rand_start);
        EventInstance {
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

    pub fn schedule_event(&mut self, event: E, primacy: PrimacyType) {
        let ev = self.events.get_mut(event.mrid());
        if let Some(ev) = ev {
            ev.primacy = primacy;
        } else {
            let ei = EventInstance::new(event, primacy);
        }
    }

    pub fn schedule_rand_event<RE: SERandomizableEvent>(
        &mut self,
        event: RE,
        primacy: PrimacyType,
    ) {
        let ev = self.events.get_mut(event.mrid());
        if let Some(ev) = ev {
            ev.primacy = primacy;
        } else {
            let ei = EventInstance::new_rand(
                primacy,
                event.randomize_duration(),
                event.randomize_start(),
                event,
            );
        }
    }
}
