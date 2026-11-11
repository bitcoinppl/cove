//! Counts blocking calls for a test harness that waits for an idle runtime
//!
//! Built only with the `test-support` feature, which only dev-dependencies enable

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// Calls started through [`super::spawn_blocking`] whose caller does not have the answer yet
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

/// Calls through [`super::spawn_blocking`] whose caller got the answer or stopped waiting for it
static ANSWERED: AtomicU64 = AtomicU64::new(0);

/// The calls through [`super::spawn_blocking`] at one moment
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockingCalls {
    /// Started calls whose caller does not have the answer yet
    pub in_flight: usize,
    /// Calls whose caller got the answer or stopped waiting, since the process started
    pub answered: u64,
}

/// The calls through [`super::spawn_blocking`] now
///
/// A test harness that waits for an idle runtime reads this: the answer of a blocking call
/// reaches its caller after the blocking thread returns, so a runtime can park between the two,
/// and only a call that is no longer in flight can't wake it again. The count is per process
pub fn blocking_calls() -> BlockingCalls {
    // in flight first: a call counts as answered before it leaves the in-flight count, so a
    // read that misses it in flight sees it answered
    let in_flight = IN_FLIGHT.load(Ordering::SeqCst);
    let answered = ANSWERED.load(Ordering::SeqCst);
    BlockingCalls { in_flight, answered }
}

/// Counts one call as in flight until its handle answers or is dropped
pub(crate) struct InFlight;

impl InFlight {
    pub(crate) fn begin() -> Self {
        IN_FLIGHT.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        ANSWERED.fetch_add(1, Ordering::SeqCst);
        IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
    }
}
