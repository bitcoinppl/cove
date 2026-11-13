//! One clock for Tokio timers and the wall-clock seconds persisted in `retry_after`

use std::time::Duration;

use cove_util::time::test_clock;

/// A fixed, recognisable start time, far from the epoch so the override is never zero
const HARNESS_EPOCH_SECS: u64 = 1_800_000_000;

#[derive(Debug)]
pub(crate) struct HarnessClock(());

impl HarnessClock {
    pub(crate) fn install() -> Self {
        test_clock::set(HARNESS_EPOCH_SECS);
        Self(())
    }

    /// The wall-clock seconds production code reads
    pub(crate) fn now_secs(&self) -> u64 {
        cove_util::time::unix_timestamp_secs_or_zero()
    }

    /// Move timers and wall-clock time forward together
    ///
    /// Wall-clock time has whole-second resolution, so sub-second advances move timers only
    pub(crate) async fn advance(&self, by: Duration) {
        test_clock::advance(by.as_secs());

        // tokio advances a paused clock only from the runtime's own thread; from the test
        // thread the parked driver would never see the new time. Its `advance` then yields to
        // the time driver, so every timer this step makes due fires before the runtime thread
        // can park, which is why settling can trust a park
        cove_tokio::task::spawn(tokio::time::advance(by)).await.expect("advance harness clock");
    }
}
