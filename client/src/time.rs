use std::time::{SystemTime, UNIX_EPOCH};

use sep2_common::packages::primitives::Int64;

// TODO: Server time sync?

pub fn current_time() -> Int64 {
    let current_time = SystemTime::now();
    let duration = current_time
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards");
    Int64(duration.as_secs() as i64)
}
