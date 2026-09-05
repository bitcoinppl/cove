use std::time::Duration;

use backon::ExponentialBuilder;

/// Backoff for the startup price and fee fetches: quick first retries, capped at five seconds
pub(crate) fn network_fetch_backoff() -> ExponentialBuilder {
    ExponentialBuilder::default()
        .with_min_delay(Duration::from_millis(10))
        .with_max_delay(Duration::from_secs(5))
        .with_max_times(20)
}
