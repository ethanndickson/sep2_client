//! Time Function Set

use std::{
    sync::atomic::AtomicI64,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use sep2_common::packages::{primitives::Int64, time::Time};

static TIME_OFFSET: AtomicI64 = AtomicI64::new(0);

/// IEEE 2030.5 Representation of SystemTime
#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord)]
pub struct SEPTime {
    inner: SystemTime,
}

impl SEPTime {
    fn now() -> SEPTime {
        SEPTime {
            inner: SystemTime::now(),
        }
    }
}

impl From<SEPTime> for SystemTime {
    fn from(value: SEPTime) -> Self {
        value.inner
    }
}

impl From<SEPTime> for Int64 {
    fn from(value: SEPTime) -> Self {
        Int64(i64::from(value))
    }
}

impl From<SEPTime> for i64 {
    fn from(value: SEPTime) -> Self {
        u64::from(value) as i64
    }
}

impl From<SEPTime> for u64 {
    fn from(value: SEPTime) -> Self {
        value
            .inner
            .duration_since(UNIX_EPOCH)
            // TODO: Can this reasonably happen?? Do we need to return a Result?
            .expect("Time went backwards")
            .as_secs()
    }
}

impl std::ops::Add<i64> for SEPTime {
    type Output = SEPTime;

    fn add(mut self, rhs: i64) -> Self::Output {
        let sign = rhs.signum();
        // Duration cannot be negative, this branching is required
        let duration = Duration::from_secs(rhs.unsigned_abs());
        if sign.is_positive() {
            self.inner += duration;
        } else {
            self.inner -= duration;
        }
        self
    }
}

/// Return the current time, as an Int64
pub fn current_time() -> SEPTime {
    SEPTime::now()
}

/// Return the current time, as an Int64, with the global time offset supplied.
pub fn current_time_with_offset() -> SEPTime {
    current_time() + TIME_OFFSET.load(std::sync::atomic::Ordering::Relaxed)
}

/// Given a Time resource, calculate it's offset from the system time,
/// and set that offset to be applied to all future calls to [`current_time_with_offset`]
pub fn update_time_offset(time: Time) {
    let offset = time.current_time.get() - i64::from(current_time());
    TIME_OFFSET.store(offset, std::sync::atomic::Ordering::Relaxed);
}

/// Intermittently sleep until the provided instant,
/// waking at an interval defined by `rate`.
/// This uses `tokio::time:sleep`, which, like `thread::sleep` does not make progress while the device itself is asleep,
/// hence the intermittent wakeups.
pub async fn sleep_until(timestamp: Instant, tickrate: Duration) {
    loop {
        tokio::time::sleep(tickrate).await;
        if Instant::now() > timestamp {
            break;
        }
    }
}

#[test]
fn septime_add() {
    let earlier = current_time() + -100i64;
    assert!(earlier < current_time());
    let later = current_time() + 100i64;
    assert!(later > current_time());
}

#[test]
fn septime_offset() {
    let mut some_time = Time::default();
    some_time.current_time = (current_time() + 100).into();
    update_time_offset(some_time);
    assert!(current_time_with_offset() > current_time());
}
