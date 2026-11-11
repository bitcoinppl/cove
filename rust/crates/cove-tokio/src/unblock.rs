/// Run `f` on a blocking thread and wait for its answer
///
/// With the `test-support` feature, the call counts in `task::blocking_calls` until the caller
/// has the answer
pub async fn run_blocking<F, R>(f: F) -> R
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    crate::task::spawn_blocking(f).await.expect("blocking task failed")
}
