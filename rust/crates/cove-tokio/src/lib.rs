mod abortable_task;
mod debounced_task;
pub mod task;
mod timeout;
pub mod unblock;

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
