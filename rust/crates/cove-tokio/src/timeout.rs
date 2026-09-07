use std::future::Future;
use std::time::Duration;
use tokio::time;

/// Blanket extension trait: implemented for *all* futures.
pub trait FutureTimeoutExt: Future + Sized {
    /// Wrap this future in a Tokio timeout.
    fn with_timeout(self, dur: Duration) -> time::Timeout<Self> {
        time::timeout(dur, self)
    }
}

impl<F: Future> FutureTimeoutExt for F {}
