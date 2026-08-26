mod abortable_task;
mod debounced_task;
pub mod task;
mod timeout;
pub mod unblock;

use std::future::Future;
use std::sync::OnceLock;

use tokio::runtime::Handle;

pub use abortable_task::AbortableTask;
pub use debounced_task::DebouncedTask;
pub use timeout::FutureTimeoutExt;

pub(crate) static TOKIO: OnceLock<Handle> = OnceLock::new();

pub fn init() {
    if is_tokio_initialized() {
        return;
    }

    let _ = TOKIO.set(Handle::current());
}

pub fn is_tokio_initialized() -> bool {
    TOKIO.get().is_some()
}

/// Failure from a synchronous call into the process Tokio runtime
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum RuntimeBridgeError {
    /// The process runtime has not been initialized
    Unavailable,
    /// Blocking from a Tokio runtime thread would panic or deadlock
    RuntimeThreadCall,
}

/// Run an async operation from a non-Tokio platform thread
///
/// # Errors
///
/// Returns [`RuntimeBridgeError::Unavailable`] before runtime initialization and
/// [`RuntimeBridgeError::RuntimeThreadCall`] when called from a Tokio runtime thread
pub fn try_block_on<T>(future: T) -> Result<T::Output, RuntimeBridgeError>
where
    T: Future + Send + 'static,
    T::Output: Send + 'static,
{
    let handle = TOKIO.get().ok_or(RuntimeBridgeError::Unavailable)?;
    if Handle::try_current().is_ok() {
        return Err(RuntimeBridgeError::RuntimeThreadCall);
    }

    Ok(handle.block_on(future))
}

#[cfg(test)]
mod tests {
    use super::{RuntimeBridgeError, init, try_block_on};

    #[test]
    fn runtime_thread_call_returns_error_without_panicking() {
        tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async {
            init();

            assert_eq!(try_block_on(async { 1_u8 }), Err(RuntimeBridgeError::RuntimeThreadCall));
        });
    }
}
