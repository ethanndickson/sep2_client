use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sep2_common::packages::primitives::Int64;

// TODO: Server time sync?
pub(crate) const SLEEP_TICKRATE: Duration = Duration::from_secs(60 * 5);

/// Return the current time, as an Int64
pub fn current_time() -> Int64 {
    let current_time = SystemTime::now();
    let duration = current_time
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards");
    Int64(duration.as_secs() as i64)
}

/// Intermittently sleep until the provided timestamp,
/// waking at an interval defined by `rate`.
pub async fn sleep_until(timestamp: i64, rate: Duration) {
    loop {
        tokio::time::sleep(rate).await;
        if current_time().get() > timestamp {
            break;
        }
    }
}
