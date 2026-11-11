use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds since the unix epoch, or `None` when the system clock is before the epoch
#[must_use]
pub fn unix_timestamp_secs() -> Option<u64> {
    #[cfg(feature = "test-clock")]
    if let Some(now) = test_clock::now() {
        return Some(now);
    }

    SystemTime::now().duration_since(UNIX_EPOCH).ok().map(|elapsed| elapsed.as_secs())
}

/// Seconds since the unix epoch, treating a clock before the epoch as zero
#[must_use]
pub fn unix_timestamp_secs_or_zero() -> u64 {
    unix_timestamp_secs().unwrap_or_default()
}

/// Process-wide wall-clock override for test harnesses that control time
#[cfg(feature = "test-clock")]
pub mod test_clock {
    use std::sync::atomic::{AtomicU64, Ordering};

    // zero means no override; no test needs the epoch itself
    static NOW_SECS: AtomicU64 = AtomicU64::new(0);

    /// Pin [`super::unix_timestamp_secs`] to `secs`
    pub fn set(secs: u64) {
        NOW_SECS.store(secs, Ordering::SeqCst);
    }

    /// Move the pinned clock forward by `secs`
    pub fn advance(secs: u64) {
        NOW_SECS.fetch_add(secs, Ordering::SeqCst);
    }

    pub(super) fn now() -> Option<u64> {
        match NOW_SECS.load(Ordering::SeqCst) {
            0 => None,
            secs => Some(secs),
        }
    }
}
