use std::future::Future;
use tokio::task::JoinHandle;

/// A task that will be cancelled (aborted) when dropped
#[derive(Debug)]
pub struct AbortableTask<T>(JoinHandle<T>);

impl<T> AbortableTask<T> {
    pub const fn new(handle: JoinHandle<T>) -> Self {
        Self(handle)
    }
}

impl<T> AbortableTask<T>
where
    T: Send + 'static,
{
    pub fn spawn<F>(fut: F) -> Self
    where
        F: Future<Output = T> + Send + 'static,
    {
        Self(crate::task::spawn(fut))
    }
}

impl<T> Drop for AbortableTask<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}
