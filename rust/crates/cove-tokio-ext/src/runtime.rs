use std::{future::Future, sync::OnceLock};

use tokio::{runtime::Handle, task::JoinHandle};

static HANDLE: OnceLock<Handle> = OnceLock::new();

pub fn init(handle: Handle) {
    let _ = HANDLE.set(handle);
}

pub fn spawn<T>(fut: T) -> JoinHandle<T::Output>
where
    T: Future + Send + 'static,
    T::Output: Send + 'static,
{
    HANDLE
        .get()
        .expect("cove-tokio-ext runtime not initialized, call cove_tokio_ext::runtime::init first")
        .spawn(fut)
}
