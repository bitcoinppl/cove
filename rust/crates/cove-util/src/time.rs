use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds since the unix epoch, or `None` when the system clock is before the epoch
#[must_use]
pub fn unix_timestamp_secs() -> Option<u64> {
    SystemTime::now().duration_since(UNIX_EPOCH).ok().map(|elapsed| elapsed.as_secs())
}

/// Seconds since the unix epoch, treating a clock before the epoch as zero
#[must_use]
pub fn unix_timestamp_secs_or_zero() -> u64 {
    unix_timestamp_secs().unwrap_or_default()
}
