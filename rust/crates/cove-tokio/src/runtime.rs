use std::{future::Future, sync::OnceLock};

use tokio::{runtime::Handle, task::JoinHandle};

pub static TOKIO: OnceLock<Handle> = OnceLock::new();

pub fn init(handle: Handle) {
    let _ = TOKIO.set(handle);
}

pub fn spawn<T>(fut: T) -> JoinHandle<T::Output>
where
    T: Future + Send + 'static,
    T::Output: Send + 'static,
{
    TOKIO
        .get()
        .expect("tokio runtime not initialized, call cove_tokio::runtime::init first")
        .spawn(fut)
}
