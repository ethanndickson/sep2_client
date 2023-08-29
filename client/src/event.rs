use common::packages::{
    objects::{Dercontrol, PrimacyType},
    traits::{SEEvent, SERandomizableEvent},
    xsd::OneHourRangeType,
};
use rand::Rng;

pub struct EventInstance<E: SEEvent, T> {
    event: E,
    start: i64,
    end: i64,
    // Function Set Specific Context
    context: Option<T>,
    primacy: PrimacyType,
}

impl<E: SEEvent, T> EventInstance<E, T> {
    pub fn new(event: E, primacy: PrimacyType) -> Self {
        let start: i64 = *event.interval().start;
        let end: i64 = start + i64::from(*event.interval().duration);
        EventInstance {
            event,
            context: None,
            primacy,
            start,
            end,
        }
    }

    pub fn new_rand(
        event: E,
        primacy: PrimacyType,
        randomize_duration: Option<OneHourRangeType>,
        randomize_start: Option<OneHourRangeType>,
    ) -> Self {
        let start: i64 = *event.interval().start + randomize(randomize_duration);
        let end: i64 = start + i64::from(*event.interval().duration) + randomize(randomize_start);
        EventInstance {
            event,
            context: None,
            primacy,
            start,
            end,
        }
    }

    pub fn supersedes(&self, other: Self) -> bool {
        self.primacy == other.primacy && self.event.creation_time() > other.event.creation_time()
            || self.primacy < other.primacy
    }
}

fn randomize(bound: Option<OneHourRangeType>) -> i64 {
    bound.map_or(0, |val| {
        let mut rng = rand::thread_rng();
        let sign = val.signum() as i64;
        rng.gen_range(0..=val.abs().into()) * sign
    })
}
