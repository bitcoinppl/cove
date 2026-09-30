//! Elapsed-time logging for slow cloud and passkey calls

use std::future::Future;
use std::time::Instant;

use tracing::info;

/// Awaits `operation`, then logs how long it took and whether it succeeded
///
/// The log line is `"{label} elapsed_ms=<ms> success=<bool>"`, so call sites keep
/// one searchable format for every timed step
pub(crate) async fn log_elapsed<T, E>(
    label: &str,
    operation: impl Future<Output = Result<T, E>>,
) -> Result<T, E> {
    let started_at = Instant::now();
    let result = operation.await;
    info!("{label} elapsed_ms={} success={}", started_at.elapsed().as_millis(), result.is_ok());

    result
}
