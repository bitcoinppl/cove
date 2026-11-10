//! Logs storage reads that fall back to a default only when they start or stop failing
//!
//! Getters such as the selected network or wallet count run on every render, so logging each
//! failed call would flood the logs while a failure that is never logged hides corruption

use std::{collections::HashSet, fmt::Display, sync::LazyLock};

use parking_lot::Mutex;
use tracing::{error, info};

/// Reads that failed on their last call, keyed by a label naming the read
static FAILING_READS: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Default::default);

/// Return the value of a read whose caller falls back to a default on failure
///
/// The first failure after a healthy read logs at ERROR, and the first success after a failure
/// logs the recovery. `read` names the read and must not contain stored values
pub(crate) fn observe<T, E: Display>(read: impl Display, result: Result<T, E>) -> Option<T> {
    match result {
        Ok(value) => {
            let mut failing = FAILING_READS.lock();
            if !failing.is_empty() && failing.remove(&read.to_string()) {
                info!("storage read {read} recovered");
            }

            Some(value)
        }

        Err(error) => {
            if FAILING_READS.lock().insert(read.to_string()) {
                error!("storage read {read} failed, using a fallback until it recovers: {error}");
            }

            None
        }
    }
}
