use std::{
    sync::{
        Arc, LazyLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use arc_swap::ArcSwap;
use backon::{ExponentialBuilder, Retryable as _};
use eyre::{Context as _, Result};
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

// Global client for getting fees
pub static FEE_CLIENT: LazyLock<FeeClient> = LazyLock::new(FeeClient::new);

pub static FEES: LazyLock<ArcSwap<Option<CachedFeeResponse>>> =
    LazyLock::new(|| ArcSwap::from_pointee(None));

pub struct FeeClient {
    url: String,
    client: reqwest::Client,
}

impl FeeClient {
    pub fn new() -> Self {
        Self::new_with_url(FEE_URL.to_string())
    }

    pub fn new_with_url(url: String) -> Self {
        Self { url, client: reqwest::Client::new() }
    }

    /// Get cached fees, will trigger background refresh if stale
    /// Returns None if no cache exists (memory or database)
    pub fn fees(&self) -> Option<FeeResponse> {
        // check in-memory cache first
        if let Some(cached) = FEES.load().as_ref() {
            let now = Instant::now();

            // cache is fresh, no refresh needed
            if now.duration_since(cached.last_fetched)
                <= Duration::from_secs(BACKGROUND_REFRESH_INTERVAL)
            {
                return Some(cached.fees);
            }

            // refresh already in flight
            if REFRESH_IN_FLIGHT
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
            {
                return Some(cached.fees);
            }

            cove_tokio::task::spawn(async move {
                if let Err(e) = fetch_and_update_fees_if_needed().await {
                    warn!("background fee refresh failed: {e:?}");
                }
                REFRESH_IN_FLIGHT.store(false, Ordering::SeqCst);
            });

            return Some(cached.fees);
        }

        // fallback to database cache
        if let Ok(Some(fees)) = Database::global().global_cache.get_fees() {
            debug!("loaded cached fees from database");
            FEES.swap(Arc::new(Some(CachedFeeResponse { fees, last_fetched: Instant::now() })));
            return Some(fees);
        }

        warn!("no cached fees found in memory or database");
        None
    }

    /// Get fees, using cache if available and fresh, otherwise fetching new
    /// Respects 30-second hard limit to prevent excessive fetching
    pub async fn fetch_and_get_fees(&self) -> Result<FeeResponse, reqwest::Error> {
        if let Some(cached) = FEES.load().as_ref() {
            let now = Instant::now();
            if now.duration_since(cached.last_fetched) < Duration::from_secs(HARD_LIMIT) {
                return Ok(cached.fees);
            }
        }

        debug!("fetching fees from network");
        let fees = self.get_new_fees().await?;
        update_fees(fees);

        Ok(fees)
    }

    /// Always gets new fees from the server
    async fn get_new_fees(&self) -> Result<FeeResponse, reqwest::Error> {
        let response = self.client.get(&self.url).send().await?;
        let fees: FeeResponse = response.json().await?;
        Ok(fees)
    }
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct FeeResponse {
    pub fastest_fee: f32,
    pub half_hour_fee: f32,
    pub hour_fee: f32,
    pub economy_fee: f32,
    pub minimum_fee: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct CachedFeeResponse {
    pub fees: FeeResponse,
    pub last_fetched: Instant,
}

/// Convert fee response to fee rate options
impl From<FeeResponse> for FeeRateOptions {
    fn from(fees: FeeResponse) -> Self {
        /// Policy minimum fee rate in sat/vb
        const POLICY_MIN_FEE_RATE: f32 = 1.0;

        /// Minimum gap between fee tiers to ensure they're visually distinct
        const TIER_GAP: f32 = 0.1;

        let min_relay_rate = fees.minimum_fee.max(POLICY_MIN_FEE_RATE);

        let slow_rate =
            f32::midpoint(fees.economy_fee, fees.hour_fee).min(fees.hour_fee).max(min_relay_rate);

        let medium_rate = fees.half_hour_fee.max(slow_rate + TIER_GAP);
        let fast_rate = fees.fastest_fee.max(medium_rate + TIER_GAP);

        let slow = FeeRateOption {
            fee_speed: FeeSpeed::Slow,
            fee_rate: FeeRate::from_sat_per_vb(slow_rate),
        };
        let medium = FeeRateOption {
            fee_speed: FeeSpeed::Medium,
            fee_rate: FeeRate::from_sat_per_vb(medium_rate),
        };
        let fast = FeeRateOption {
            fee_speed: FeeSpeed::Fast,
            fee_rate: FeeRate::from_sat_per_vb(fast_rate),
        };

        Self { fast, medium, slow }
    }
}

/// get and update fees
pub async fn get_and_update_fees() -> Result<(), reqwest::Error> {
    let fees = FEE_CLIENT.get_new_fees().await?;
    update_fees(fees);
    Ok(())
}

/// Update fees in memory cache and database
fn update_fees(fees: FeeResponse) {
    debug!("update_fees");
    let cached = CachedFeeResponse { fees, last_fetched: Instant::now() };
    FEES.swap(Arc::new(Some(cached)));
    Updater::send_update(AppMessage::FeesChanged(fees));

    // persist to database
    let db = Database::global();
    if let Err(e) = db.global_cache.set_fees(fees) {
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

    // try loading from database first
    if let Ok(Some(fees)) = Database::global().global_cache.get_fees() {
        debug!("loaded fees from database cache");
        FEES.swap(Arc::new(Some(CachedFeeResponse { fees, last_fetched: Instant::now() })));
        Updater::send_update(AppMessage::FeesChanged(fees));
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
        Ok(fees) => update_fees(fees),
        Err(error) => warn!("unable to get fees: {error:?}"),
    }
}

/// Fetch and update fees if needed (respects hard limit)
pub async fn fetch_and_update_fees_if_needed() -> Result<()> {
    if let Some(cached) = FEES.load().as_ref() {
        let now = Instant::now();
        if now.duration_since(cached.last_fetched) < Duration::from_secs(HARD_LIMIT) {
            return Ok(());
        }
    }

    debug!("fetching fees");
    let fees = FEE_CLIENT.get_new_fees().await.context("unable to get fees")?;
    update_fees(fees);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn low_fees_reported_bug() {
        // API returns sub-1 fees, app should show 1.0, 1.1, 1.2
        let fees = fee_response(0.1, 0.5, 0.5, 1.0);
        let options = FeeRateOptions::from(fees);

        assert_eq!(options.slow.fee_rate.to_sat_per_kwu(), 250); // 1.0 sat/vb
        assert_eq!(options.medium.fee_rate.to_sat_per_kwu(), 275); // 1.1 sat/vb
        assert_eq!(options.fast.fee_rate.to_sat_per_kwu(), 300); // 1.2 sat/vb
    }

    #[test]
    fn all_fees_below_one() {
        let fees = fee_response(0.1, 0.1, 0.1, 0.1);
        let options = FeeRateOptions::from(fees);

        assert_eq!(options.slow.fee_rate.to_sat_per_kwu(), 250); // 1.0 sat/vb
        assert_eq!(options.medium.fee_rate.to_sat_per_kwu(), 275); // 1.1 sat/vb
        assert_eq!(options.fast.fee_rate.to_sat_per_kwu(), 300); // 1.2 sat/vb
    }

    #[test]
    fn all_fees_equal_at_one() {
        let fees = fee_response(1.0, 1.0, 1.0, 1.0);
        let options = FeeRateOptions::from(fees);

        assert_eq!(options.slow.fee_rate.to_sat_per_kwu(), 250); // 1.0 sat/vb
        assert_eq!(options.medium.fee_rate.to_sat_per_kwu(), 275); // 1.1 sat/vb
        assert_eq!(options.fast.fee_rate.to_sat_per_kwu(), 300); // 1.2 sat/vb
    }

    #[test]
    fn normal_differentiated_fees() {
        // high enough fees should pass through without inflation
        let fees = fee_response(5.0, 10.0, 15.0, 20.0);
        let options = FeeRateOptions::from(fees);

        assert_eq!(options.slow.fee_rate.to_sat_per_kwu(), 1875); // 7.5 sat/vb
        assert_eq!(options.medium.fee_rate.to_sat_per_kwu(), 3750); // 15.0 sat/vb
        assert_eq!(options.fast.fee_rate.to_sat_per_kwu(), 5000); // 20.0 sat/vb
    }

    #[test]
    fn fees_slightly_above_one() {
        let fees = fee_response(1.0, 2.0, 3.0, 5.0);
        let options = FeeRateOptions::from(fees);

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
        let options = FeeRateOptions::from(fees);

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
        let options = FeeRateOptions::from(fees);

        assert_eq!(options.slow.fee_rate.to_sat_per_kwu(), 375); // 1.5 sat/vb (floor)
        assert_eq!(options.medium.fee_rate.to_sat_per_kwu(), 750); // 3.0 sat/vb
        assert_eq!(options.fast.fee_rate.to_sat_per_kwu(), 1250); // 5.0 sat/vb
    }
}
