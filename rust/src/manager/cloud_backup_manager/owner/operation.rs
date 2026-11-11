//! Identity of a runtime Cloud Backup operation
//!
//! Every exclusive operation the supervisor starts gets a fresh `OperationId`, and every
//! completion carries the id of the operation it belongs to. A completion whose id is no longer
//! the running operation's is stale: the supervisor discards it with a `debug!` log and never
//! reports it to the user as a rejection

use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_OPERATION_ID: AtomicU64 = AtomicU64::new(0);

/// Identity of one exclusive operation run
///
/// Ids are unique for the life of the process. They are never persisted, because PLAN §8.6
/// rules out a durable generation counter; a relaunch starts no operation from a stored id
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct OperationId(u64);

impl OperationId {
    /// Allocates the id for a new operation
    ///
    /// Only the supervisor starts operations, so it is the only production caller
    pub(crate) fn next() -> Self {
        Self(NEXT_OPERATION_ID.fetch_add(1, Ordering::Relaxed))
    }
}

impl std::fmt::Display for OperationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "op#{}", self.0)
    }
}

pub(crate) mod test_support {
    #![cfg(test)]

    use super::OperationId;

    /// A fixed id, for tests that compare claims built with known ids
    pub(crate) fn operation_id(raw: u64) -> OperationId {
        OperationId(raw)
    }
}
