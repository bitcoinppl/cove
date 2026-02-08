mod abortable_task;
mod debounced_task;
pub mod runtime;
pub mod task;
pub mod unblock;

pub use abortable_task::AbortableTask;
pub use debounced_task::DebouncedTask;

use std::future::Future;
use std::time::Duration;
use tokio::time;

/// Blanket extension trait: implemented for *all* futures.
pub trait FutureTimeoutExt: Future + Sized {
    /// Wrap this future in a Tokio timeout.
    fn with_timeout(self, dur: Duration) -> time::Timeout<Self> {
        time::timeout(dur, self)
    }

    /// Wrap this future in a timeout that ends at an absolute deadline.
    fn with_deadline(self, deadline: time::Instant) -> time::Timeout<Self> {
        time::timeout_at(deadline, self)
    }
}

impl<F: Future> FutureTimeoutExt for F {}
