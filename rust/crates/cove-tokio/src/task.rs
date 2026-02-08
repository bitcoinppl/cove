use crate::TOKIO;

use act_zero::{Actor, Addr};
use core::future::Future;
use futures::task::{Spawn, SpawnError};
use tokio::task::JoinHandle;

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

#[allow(dead_code)]
pub fn block_on<T>(task: T) -> T::Output
where
    T: Future + Send + 'static,
    T::Output: Send + 'static,
{
    let handle = TOKIO.get().expect("tokio runtime not initialized");
    handle.block_on(task)
}

pub fn spawn_blocking<F, R>(f: F) -> JoinHandle<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    TOKIO.get().expect("tokio runtime not initialized").spawn_blocking(f)
}

/// Provides an infallible way to spawn an actor onto the Tokio runtime,
/// equivalent to `Addr::new`.
pub fn spawn_actor<T: Actor>(actor: T) -> Addr<T> {
    Addr::new(&CustomRuntime, actor).unwrap()
}
