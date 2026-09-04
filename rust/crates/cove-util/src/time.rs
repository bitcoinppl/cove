use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds since the unix epoch, or `None` when the system clock is before the epoch
#[must_use]
pub fn unix_timestamp_secs() -> Option<u64> {
    SystemTime::now().duration_since(UNIX_EPOCH).ok().map(|elapsed| elapsed.as_secs())
}
