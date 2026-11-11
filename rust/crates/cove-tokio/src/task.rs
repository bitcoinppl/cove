use crate::TOKIO;

use std::pin::Pin;
use std::task::{Context, Poll};

use act_zero::{Actor, Addr};
use core::future::Future;
use futures::task::{Spawn, SpawnError};
use tokio::task::{JoinError, JoinHandle};

#[cfg(feature = "test-support")]
mod blocking_calls;

#[cfg(feature = "test-support")]
use blocking_calls::InFlight;
#[cfg(feature = "test-support")]
pub use blocking_calls::{BlockingCalls, blocking_calls};

struct CustomRuntime;

impl Spawn for CustomRuntime {
    fn spawn_obj(&self, future: futures::future::FutureObj<'static, ()>) -> Result<(), SpawnError> {
        spawn(future);
        Ok(())
    }
}

pub fn spawn<T>(task: T) -> JoinHandle<T::Output>
where
    T: Future + Send + 'static,
    T::Output: Send + 'static,
{
    TOKIO.get().expect("tokio runtime not initialized").spawn(task)
}

pub fn block_on<T>(task: T) -> T::Output
where
    T: Future + Send + 'static,
    T::Output: Send + 'static,
{
    let handle = TOKIO.get().expect("tokio runtime not initialized");
    handle.block_on(task)
}

/// Run `f` on a blocking thread; the handle answers with its result
///
/// With the `test-support` feature, the call counts in `blocking_calls` until its handle answers
/// or is dropped, which is later than the blocking thread returning
pub fn spawn_blocking<F, R>(f: F) -> BlockingHandle<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    #[cfg(feature = "test-support")]
    let call = InFlight::begin();
    let handle = TOKIO.get().expect("tokio runtime not initialized").spawn_blocking(f);

    BlockingHandle {
        handle,
        #[cfg(feature = "test-support")]
        _call: call,
    }
}

/// A blocking call started by [`spawn_blocking`], awaited like a [`JoinHandle`]
///
/// With the `test-support` feature, it counts as in flight until it answers its caller or is
/// dropped
#[must_use = "a blocking call's handle answers with its result"]
pub struct BlockingHandle<R> {
    handle: JoinHandle<R>,
    #[cfg(feature = "test-support")]
    _call: InFlight,
}

impl<R> Future for BlockingHandle<R> {
    type Output = Result<R, JoinError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.get_mut().handle).poll(cx)
    }
}

/// Log a join failure at the task owner unless it is the normal cancellation of an aborted task
///
/// Release builds abort on panic, so this mostly surfaces panics in debug and test builds; the
/// panic hook records the location either way
pub fn log_unexpected_join_error(operation: &str, error: &JoinError) {
    if error.is_cancelled() {
        return;
    }

    // the join error's Display carries the panic payload, which can hold secrets
    tracing::error!("{operation} task panicked");
}

/// Provides an infallible way to spawn an actor onto the Tokio runtime,
/// equivalent to `Addr::new`.
pub fn spawn_actor<T: Actor>(actor: T) -> Addr<T> {
    Addr::new(&CustomRuntime, actor).unwrap()
}
