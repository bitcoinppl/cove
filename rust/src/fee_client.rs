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

            crate::task::spawn(async move {
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
        let (slow_rate, slow) = {
            // slow rate is the between economy and hour fees
            let rate = f32::midpoint(fees.economy_fee, fees.hour_fee);

            // rate should never be more than the hour fee
            let rate = rate.min(fees.hour_fee);

            // slow rate should never be less than or the same as the minimum fee
            let rate = rate.max(fees.minimum_fee + 1.1);

            (
                rate,
                FeeRateOption {
                    fee_speed: FeeSpeed::Slow,
                    fee_rate: FeeRate::from_sat_per_vb(rate),
                },
            )
        };

        let (medium_rate, medium) = {
            let rate = fees.half_hour_fee.max(slow_rate + 1.1);
            (
                rate,
                FeeRateOption {
                    fee_speed: FeeSpeed::Medium,
                    fee_rate: FeeRate::from_sat_per_vb(rate),
                },
            )
        };

        let fast = {
            let rate = fees.fastest_fee.max(medium_rate + 1.1);
            FeeRateOption { fee_speed: FeeSpeed::Fast, fee_rate: FeeRate::from_sat_per_vb(rate) }
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
