use std::{
    sync::{Arc, LazyLock},
    time::{Duration, Instant},
};

use arc_swap::ArcSwap;
use backon::{ExponentialBuilder, Retryable as _};
use eyre::Result;
use tracing::warn;

use cove_types::fees::{FeeRate, FeeRateOption, FeeRateOptions, FeeSpeed};

const FEE_URL: &str = "https://mempool.space/api/v1/fees/recommended";

const ONE_MIN: u64 = 60;
// Global client for getting prices
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

    /// Always returns the cached fees, will also update the fees cache in the background if needed
    pub fn fees(&self) -> Option<FeeResponse> {
        if let Some(cached) = FEES.load().as_ref() {
            let now = Instant::now();
            if now.duration_since(cached.last_fetched) > std::time::Duration::from_secs(ONE_MIN) {
                crate::task::spawn(async move { FEE_CLIENT.fetch_and_get_fees().await });
            }

            return Some(cached.fees);
        }

        None
    }

    /// Get fees from the memory cache if it exists and is less than 60 seconds old
    /// otherwise get the new fees from the server
    pub async fn fetch_and_get_fees(&self) -> Result<FeeResponse, reqwest::Error> {
        if let Some(cached) = FEES.load().as_ref() {
            let now = Instant::now();
            if now.duration_since(cached.last_fetched) < std::time::Duration::from_secs(ONE_MIN) {
                return Ok(cached.fees);
            }
        }

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

#[derive(Debug, Clone, Copy, serde::Deserialize, uniffi::Record)]
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
            let rate = (fees.economy_fee + fees.hour_fee) / 2.0;

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

/// update price in cache
fn update_fees(fees: FeeResponse) {
    let cached = CachedFeeResponse { fees, last_fetched: Instant::now() };

    FEES.swap(Arc::new(Some(cached)));
}

// init fees
pub async fn init_fees() {
    if FEES.load().as_ref().is_some() {
        warn!("fees already initialized");
        return;
    }

    let result = (|| FEE_CLIENT.fetch_and_get_fees())
        .retry(
            ExponentialBuilder::default()
                .with_min_delay(Duration::from_millis(10))
                .with_max_delay(Duration::from_secs(5))
                .with_max_times(20),
        )
        .await;

    if let Err(error) = result {
        warn!("unable to get fees: {error:?}");
    }
}
