use std::{
    sync::{
        Arc, LazyLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use arc_swap::ArcSwap;
use backon::{ExponentialBuilder, Retryable as _};
use eyre::{Context as _, Result};
use once_cell::sync::OnceCell;
use tracing::{debug, error, warn};

/// Guard to prevent multiple concurrent background refresh tasks
static REFRESH_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

use crate::{
    app::reconcile::{AppStateReconcileMessage as AppMessage, Updater},
    database::Database,
};
use cove_types::fees::{FeeRate, FeeRateOption, FeeRateOptions, FeeSpeed};

const FEE_URL: &str = "https://mempool.space/api/v1/fees/recommended";

/// Background refresh interval in seconds
const BACKGROUND_REFRESH_INTERVAL: u64 = 60;

/// Hard limit: never fetch if < 30 seconds since last fetch
const HARD_LIMIT: u64 = 30;

/// Do not use a persisted fee snapshot as a fallback after this amount of time
const STALE_FALLBACK_MAX_AGE: Duration = Duration::from_secs(60 * 60);

/// Maximum fee rate accepted from the remote fee service, in sat/vB
const MAX_REMOTE_FEE_RATE: f32 = 500.0;

// Global client for getting fees
pub static FEE_CLIENT: LazyLock<FeeClient> = LazyLock::new(FeeClient::new);

pub static FEES: LazyLock<ArcSwap<Option<CachedFeeResponse>>> =
    LazyLock::new(|| ArcSwap::from_pointee(None));

pub struct FeeClient {
    url: String,
    client: OnceCell<reqwest::Client>,
}

/// Errors raised while validating fee data from the remote service
#[derive(Debug, Clone, thiserror::Error)]
pub enum FeeValidationError {
    #[error(
        "{field} fee rate must be finite, positive, and at most {MAX_REMOTE_FEE_RATE} sat/vB (got {value})"
    )]
    InvalidRate { field: &'static str, value: f32 },
}

/// Errors raised while fetching or validating fee data
#[derive(Debug, thiserror::Error)]
pub enum FeeClientError {
    #[error(transparent)]
    Request(#[from] reqwest::Error),

    #[error(transparent)]
    Validation(#[from] FeeValidationError),
}

/// Wall-clock time at which a fee snapshot was fetched
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FeeFetchedAt(u64);

impl FeeFetchedAt {
    /// Create a timestamp from Unix seconds
    pub const fn from_unix_seconds(seconds: u64) -> Self {
        Self(seconds)
    }

    /// Return the timestamp as Unix seconds
    pub const fn as_unix_seconds(self) -> u64 {
        self.0
    }

    fn now() -> Self {
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or_default();

        Self::from_unix_seconds(seconds)
    }

    fn age(self) -> Option<Duration> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
        Some(Duration::from_secs(now.checked_sub(self.as_unix_seconds())?))
    }
}

/// A fee response and its wall-clock fetch time
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FeeSnapshot {
    pub fees: FeeResponse,
    pub fetched_at: FeeFetchedAt,
}

impl FeeSnapshot {
    fn new(fees: FeeResponse) -> Self {
        Self { fees, fetched_at: FeeFetchedAt::now() }
    }

    fn is_usable_fallback(self) -> bool {
        self.fetched_at.age().is_some_and(|age| age <= STALE_FALLBACK_MAX_AGE)
    }
}

#[derive(Debug, Clone, Copy)]
struct ValidatedFeeResponse(FeeResponse);

impl FeeClient {
    pub fn new() -> Self {
        Self::new_with_url(FEE_URL.to_string())
    }

    pub fn new_with_url(url: String) -> Self {
        Self { url, client: OnceCell::new() }
    }

    /// Get cached fees, will trigger background refresh if stale
    /// Returns None if no cache exists (memory or database)
    pub fn fees(&self) -> Option<FeeResponse> {
        if let Some(cached) = self.cached_fees() {
            if cached.snapshot().is_usable_fallback() {
                if cached.last_fetched.elapsed() <= Duration::from_secs(BACKGROUND_REFRESH_INTERVAL)
                {
                    return Some(cached.fees);
                }

                self.spawn_background_refresh();
                return Some(cached.fees);
            }

            self.spawn_background_refresh();
        }

        warn!("no usable cached fees found in memory or database");
        None
    }

    /// Get fees, using cache if available and fresh, otherwise fetching new
    /// Respects 30-second hard limit to prevent excessive fetching
    pub async fn fetch_and_get_fees(&self) -> Result<FeeResponse, FeeClientError> {
        let cached = self.cached_fees();
        if let Some(cached) = cached
            && cached.snapshot().is_usable_fallback()
            && cached.last_fetched.elapsed() < Duration::from_secs(HARD_LIMIT)
        {
            return Ok(cached.fees);
        }

        debug!("fetching fees from network");
        match self.get_new_fees().await {
            Ok(fees) => {
                let fees = fees.0;
                update_fees(fees);

                Ok(fees)
            }
            Err(error) => {
                if let Some(cached) = cached.filter(|cached| cached.snapshot().is_usable_fallback())
                {
                    warn!("fee refresh failed, using cached fees: {error}");
                    return Ok(cached.fees);
                }

                Err(error)
            }
        }
    }

    /// Always gets new fees from the server
    async fn get_new_fees(&self) -> Result<ValidatedFeeResponse, FeeClientError> {
        let response = self.client()?.get(&self.url).send().await?.error_for_status()?;
        let fees: FeeResponse = response.json().await?;
        Ok(fees.try_into()?)
    }

    fn client(&self) -> Result<&reqwest::Client, reqwest::Error> {
        self.client.get_or_try_init(cove_http::new_client)
    }

    fn cached_fees(&self) -> Option<CachedFeeResponse> {
        if let Some(cached) = FEES.load().as_ref() {
            let cached = *cached;
            if ValidatedFeeResponse::try_from(cached.fees).is_ok() {
                return Some(cached);
            }

            warn!("ignoring invalid fee snapshot from memory");
            FEES.swap(Arc::new(None));
        }

        let snapshot = match Database::global().global_cache.get_fee_snapshot() {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => return None,
            Err(error) => {
                warn!("unable to load fee snapshot from database: {error}");
                return None;
            }
        };

        if let Err(error) = ValidatedFeeResponse::try_from(snapshot.fees) {
            warn!("ignoring invalid fee snapshot from database: {error}");
            return None;
        }

        let cached = CachedFeeResponse::from_persisted_snapshot(snapshot);
        debug!("loaded cached fees from database");
        FEES.swap(Arc::new(Some(cached)));
        Some(cached)
    }

    fn spawn_background_refresh(&self) {
        if REFRESH_IN_FLIGHT
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }

        cove_tokio::task::spawn(async move {
            if let Err(error) = fetch_and_update_fees_if_needed().await {
                warn!("background fee refresh failed: {error:?}");
            }
            REFRESH_IN_FLIGHT.store(false, Ordering::SeqCst);
        });
    }
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct FeeResponse {
    pub fastest_fee: f32,
    pub half_hour_fee: f32,
    pub hour_fee: f32,
    pub economy_fee: f32,
    pub minimum_fee: f32,
}

impl TryFrom<FeeResponse> for ValidatedFeeResponse {
    type Error = FeeValidationError;

    fn try_from(fees: FeeResponse) -> Result<Self, Self::Error> {
        for (field, value) in [
            ("fastestFee", fees.fastest_fee),
            ("halfHourFee", fees.half_hour_fee),
            ("hourFee", fees.hour_fee),
            ("economyFee", fees.economy_fee),
            ("minimumFee", fees.minimum_fee),
        ] {
            if !value.is_finite() || value <= 0.0 || value > MAX_REMOTE_FEE_RATE {
                return Err(FeeValidationError::InvalidRate { field, value });
            }
        }

        let options = derive_fee_rate_options(fees);
        for (field, value) in [
            ("slow", options.slow.fee_rate.sat_per_vb()),
            ("medium", options.medium.fee_rate.sat_per_vb()),
            ("fast", options.fast.fee_rate.sat_per_vb()),
        ] {
            if !value.is_finite() || value <= 0.0 || value > MAX_REMOTE_FEE_RATE {
                return Err(FeeValidationError::InvalidRate { field, value });
            }
        }

        Ok(Self(fees))
    }
}

impl ValidatedFeeResponse {
    fn fee_rate_options(self) -> FeeRateOptions {
        derive_fee_rate_options(self.0)
    }
}

impl FeeResponse {
    /// Convert a validated remote fee response into display and builder fee tiers
    pub fn fee_rate_options(self) -> Result<FeeRateOptions, FeeValidationError> {
        Ok(ValidatedFeeResponse::try_from(self)?.fee_rate_options())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CachedFeeResponse {
    pub fees: FeeResponse,
    pub last_fetched: Instant,
    pub fetched_at: FeeFetchedAt,
}

impl CachedFeeResponse {
    fn from_fresh_snapshot(snapshot: FeeSnapshot) -> Self {
        Self { fees: snapshot.fees, last_fetched: Instant::now(), fetched_at: snapshot.fetched_at }
    }

    fn from_persisted_snapshot(snapshot: FeeSnapshot) -> Self {
        let now = Instant::now();
        let last_fetched =
            snapshot.fetched_at.age().and_then(|age| now.checked_sub(age)).unwrap_or(now);

        Self { fees: snapshot.fees, last_fetched, fetched_at: snapshot.fetched_at }
    }

    fn snapshot(self) -> FeeSnapshot {
        FeeSnapshot { fees: self.fees, fetched_at: self.fetched_at }
    }
}

fn derive_fee_rate_options(fees: FeeResponse) -> FeeRateOptions {
    /// Policy minimum fee rate in sat/vB
    const POLICY_MIN_FEE_RATE: f32 = 1.0;

    /// Minimum gap between fee tiers to ensure they're visually distinct
    const TIER_GAP: f32 = 0.1;

    let min_relay_rate = fees.minimum_fee.max(POLICY_MIN_FEE_RATE);

    let slow_rate =
        f32::midpoint(fees.economy_fee, fees.hour_fee).min(fees.hour_fee).max(min_relay_rate);

    let medium_rate = fees.half_hour_fee.max(slow_rate + TIER_GAP);
    let fast_rate = fees.fastest_fee.max(medium_rate + TIER_GAP);

    let slow =
        FeeRateOption { fee_speed: FeeSpeed::Slow, fee_rate: FeeRate::from_sat_per_vb(slow_rate) };
    let medium = FeeRateOption {
        fee_speed: FeeSpeed::Medium,
        fee_rate: FeeRate::from_sat_per_vb(medium_rate),
    };
    let fast =
        FeeRateOption { fee_speed: FeeSpeed::Fast, fee_rate: FeeRate::from_sat_per_vb(fast_rate) };

    FeeRateOptions { fast, medium, slow }
}

impl TryFrom<FeeResponse> for FeeRateOptions {
    type Error = FeeValidationError;

    fn try_from(fees: FeeResponse) -> Result<Self, Self::Error> {
        ValidatedFeeResponse::try_from(fees).map(ValidatedFeeResponse::fee_rate_options)
    }
}

/// get and update fees
pub async fn get_and_update_fees() -> Result<(), FeeClientError> {
    let fees = FEE_CLIENT.get_new_fees().await?.0;
    update_fees(fees);
    Ok(())
}

/// Update fees in memory cache and database
fn update_fees(fees: FeeResponse) {
    debug!("update_fees");
    let snapshot = FeeSnapshot::new(fees);
    let cached = CachedFeeResponse::from_fresh_snapshot(snapshot);
    FEES.swap(Arc::new(Some(cached)));
    Updater::send_update(AppMessage::FeesChanged(fees));

    // persist to database
    let db = Database::global();
    if let Err(e) = db.global_cache.set_fee_snapshot(snapshot) {
        error!("unable to save fees to database: {e:?}");
    }
}

/// Initialize fees from database cache or network
pub async fn init_and_update_fees() {
    debug!("init_fees");

    if FEES.load().as_ref().is_some() {
        warn!("fees already initialized");
        return;
    }

    // try loading a timestamped database snapshot first
    if let Some(cached) =
        FEE_CLIENT.cached_fees().filter(|cached| cached.snapshot().is_usable_fallback())
    {
        debug!("loaded fees from database cache");
        Updater::send_update(AppMessage::FeesChanged(cached.fees));
    }

    // fetch from network
    let result = (|| FEE_CLIENT.fetch_and_get_fees())
        .retry(
            ExponentialBuilder::default()
                .with_min_delay(Duration::from_millis(10))
                .with_max_delay(Duration::from_secs(5))
                .with_max_times(20),
        )
        .await;

    match result {
        Ok(_) => {}
        Err(error) => warn!("unable to get fees: {error:?}"),
    }
}

/// Fetch and update fees if needed (respects hard limit)
pub async fn fetch_and_update_fees_if_needed() -> Result<()> {
    if let Some(cached) = FEE_CLIENT.cached_fees()
        && cached.snapshot().is_usable_fallback()
        && cached.last_fetched.elapsed() < Duration::from_secs(HARD_LIMIT)
    {
        return Ok(());
    }

    debug!("fetching fees");
    let fees = FEE_CLIENT.get_new_fees().await.context("unable to get fees")?.0;
    update_fees(fees);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{io::AsyncReadExt as _, io::AsyncWriteExt as _, net::TcpListener};

    fn fee_response(economy: f32, hour: f32, half_hour: f32, fastest: f32) -> FeeResponse {
        FeeResponse {
            fastest_fee: fastest,
            half_hour_fee: half_hour,
            hour_fee: hour,
            economy_fee: economy,
            minimum_fee: economy,
        }
    }

    #[test]
    fn remote_fee_rates_must_be_finite_positive_and_bounded() {
        for invalid in [
            fee_response(0.0, 1.0, 1.0, 1.0),
            fee_response(f32::NAN, 1.0, 1.0, 1.0),
            fee_response(1.0, f32::INFINITY, 1.0, 1.0),
            fee_response(1.0, 1.0, 501.0, 1.0),
        ] {
            assert!(ValidatedFeeResponse::try_from(invalid).is_err());
        }
    }

    #[test]
    fn derived_fee_rates_are_bounded_after_tier_separation() {
        let fees = fee_response(499.9, 499.9, 499.9, 499.9);

        assert!(matches!(
            ValidatedFeeResponse::try_from(fees),
            Err(FeeValidationError::InvalidRate { field: "medium" | "fast", .. })
        ));
    }

    #[test]
    fn legacy_timestamp_is_not_a_usable_snapshot() {
        let now = UNIX_EPOCH.elapsed().expect("system clock is after Unix epoch").as_secs();
        let snapshot = FeeSnapshot {
            fees: fee_response(1.0, 1.0, 1.0, 1.0),
            fetched_at: FeeFetchedAt::from_unix_seconds(now.saturating_sub(3_601)),
        };

        assert!(!snapshot.is_usable_fallback());
    }

    #[test]
    fn persisted_snapshot_keeps_wall_clock_age_for_refresh_limits() {
        let now = UNIX_EPOCH.elapsed().expect("system clock is after Unix epoch").as_secs();
        let snapshot = FeeSnapshot {
            fees: fee_response(1.0, 1.0, 1.0, 1.0),
            fetched_at: FeeFetchedAt::from_unix_seconds(now.saturating_sub(120)),
        };

        let cached = CachedFeeResponse::from_persisted_snapshot(snapshot);

        assert!(cached.last_fetched.elapsed() >= Duration::from_secs(30));
    }

    #[tokio::test]
    async fn fee_client_rejects_http_error_status() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener binds");
        let address = listener.local_addr().expect("listener has an address");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("request arrives");
            let mut request = [0; 1024];
            let _ = stream.read(&mut request).await;
            stream
                .write_all(
                    b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .expect("response writes");
        });

        let client = FeeClient::new_with_url(format!("http://{address}"));
        let error = client.get_new_fees().await.expect_err("HTTP errors are rejected");

        assert!(matches!(error, FeeClientError::Request(_)));
        server.await.expect("server exits");
    }

    #[test]
    fn low_fees_reported_bug() {
        // API returns sub-1 fees, app should show 1.0, 1.1, 1.2
        let fees = fee_response(0.1, 0.5, 0.5, 1.0);
        let options = fees.fee_rate_options().expect("fee rates are valid");

        assert_eq!(options.slow.fee_rate.to_sat_per_kwu(), 250); // 1.0 sat/vb
        assert_eq!(options.medium.fee_rate.to_sat_per_kwu(), 275); // 1.1 sat/vb
        assert_eq!(options.fast.fee_rate.to_sat_per_kwu(), 300); // 1.2 sat/vb
    }

    #[test]
    fn all_fees_below_one() {
        let fees = fee_response(0.1, 0.1, 0.1, 0.1);
        let options = fees.fee_rate_options().expect("fee rates are valid");

        assert_eq!(options.slow.fee_rate.to_sat_per_kwu(), 250); // 1.0 sat/vb
        assert_eq!(options.medium.fee_rate.to_sat_per_kwu(), 275); // 1.1 sat/vb
        assert_eq!(options.fast.fee_rate.to_sat_per_kwu(), 300); // 1.2 sat/vb
    }

    #[test]
    fn all_fees_equal_at_one() {
        let fees = fee_response(1.0, 1.0, 1.0, 1.0);
        let options = fees.fee_rate_options().expect("fee rates are valid");

        assert_eq!(options.slow.fee_rate.to_sat_per_kwu(), 250); // 1.0 sat/vb
        assert_eq!(options.medium.fee_rate.to_sat_per_kwu(), 275); // 1.1 sat/vb
        assert_eq!(options.fast.fee_rate.to_sat_per_kwu(), 300); // 1.2 sat/vb
    }

    #[test]
    fn normal_differentiated_fees() {
        // high enough fees should pass through without inflation
        let fees = fee_response(5.0, 10.0, 15.0, 20.0);
        let options = fees.fee_rate_options().expect("fee rates are valid");

        assert_eq!(options.slow.fee_rate.to_sat_per_kwu(), 1875); // 7.5 sat/vb
        assert_eq!(options.medium.fee_rate.to_sat_per_kwu(), 3750); // 15.0 sat/vb
        assert_eq!(options.fast.fee_rate.to_sat_per_kwu(), 5000); // 20.0 sat/vb
    }

    #[test]
    fn fees_slightly_above_one() {
        let fees = fee_response(1.0, 2.0, 3.0, 5.0);
        let options = fees.fee_rate_options().expect("fee rates are valid");

        assert_eq!(options.slow.fee_rate.to_sat_per_kwu(), 375); // 1.5 sat/vb
        assert_eq!(options.medium.fee_rate.to_sat_per_kwu(), 750); // 3.0 sat/vb
        assert_eq!(options.fast.fee_rate.to_sat_per_kwu(), 1250); // 5.0 sat/vb
    }

    #[test]
    fn minimum_fee_above_floor() {
        // API's minimum_fee is 2.0, higher than MIN_FEE_RATE (1.0)
        // all tiers should respect this higher minimum
        let fees = FeeResponse {
            fastest_fee: 2.0,
            half_hour_fee: 2.0,
            hour_fee: 2.0,
            economy_fee: 2.0,
            minimum_fee: 2.0,
        };
        let options = fees.fee_rate_options().expect("fee rates are valid");

        assert_eq!(options.slow.fee_rate.to_sat_per_kwu(), 500); // 2.0 sat/vb
        assert_eq!(options.medium.fee_rate.to_sat_per_kwu(), 525); // 2.1 sat/vb
        assert_eq!(options.fast.fee_rate.to_sat_per_kwu(), 550); // 2.2 sat/vb
    }

    #[test]
    fn minimum_fee_lifts_slow_above_raw() {
        // economy and hour are below minimum_fee
        // slow should be lifted to minimum_fee, not midpoint
        let fees = FeeResponse {
            fastest_fee: 5.0,
            half_hour_fee: 3.0,
            hour_fee: 1.0,
            economy_fee: 0.5,
            minimum_fee: 1.5,
        };
        let options = fees.fee_rate_options().expect("fee rates are valid");

        assert_eq!(options.slow.fee_rate.to_sat_per_kwu(), 375); // 1.5 sat/vb (floor)
        assert_eq!(options.medium.fee_rate.to_sat_per_kwu(), 750); // 3.0 sat/vb
        assert_eq!(options.fast.fee_rate.to_sat_per_kwu(), 1250); // 5.0 sat/vb
    }
}
