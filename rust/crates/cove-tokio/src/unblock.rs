pub async fn run_blocking<F, R>(f: F) -> R
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    crate::task::spawn_blocking(f).await.expect("blocking task failed")
}
