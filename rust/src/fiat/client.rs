use std::{
    sync::{Arc, LazyLock},
    time::Duration,
};

use arc_swap::ArcSwap;
use eyre::{Context as _, Result};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use tracing::{error, warn};

use crate::{database::Database, fiat::FiatCurrency, transaction::Amount};
use macros::impl_default_for;

const CURRENCY_URL: &str = "https://mempool.space/api/v1/prices";

const ONE_MIN: u64 = 60;

// Global client for getting prices
pub static FIAT_CLIENT: LazyLock<FiatClient> = LazyLock::new(FiatClient::new);

pub static PRICES: LazyLock<ArcSwap<Option<PriceResponse>>> =
    LazyLock::new(|| ArcSwap::from_pointee(None));

#[derive(Debug, Clone, uniffi::Object)]
pub struct FiatClient {
    url: String,
    client: reqwest::Client,
    wait_before_new_prices: u64,
}

#[derive(
    Debug, Copy, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, uniffi::Object,
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub struct PriceResponse {
    #[serde(rename = "time")]
    pub time: u64,
    pub usd: u64,
    pub eur: u64,
    pub gbp: u64,
    pub cad: u64,
    pub chf: u64,
    pub aud: u64,
    pub jpy: u64,
}

#[uniffi::export]
impl PriceResponse {
    pub fn get(&self) -> u64 {
        let currency = Database::global()
            .global_config
            .fiat_currency()
            .unwrap_or_default();

        self.get_for_currency(currency)
    }

    pub fn get_for_currency(&self, currency: FiatCurrency) -> u64 {
        match currency {
            FiatCurrency::Usd => self.usd,
            FiatCurrency::Eur => self.eur,
            FiatCurrency::Gbp => self.gbp,
            FiatCurrency::Cad => self.cad,
            FiatCurrency::Chf => self.chf,
            FiatCurrency::Aud => self.aud,
            FiatCurrency::Jpy => self.jpy,
        }
    }
}

impl_default_for!(FiatClient);

impl FiatClient {
    fn new() -> Self {
        Self {
            url: CURRENCY_URL.to_string(),
            client: reqwest::Client::new(),
            wait_before_new_prices: ONE_MIN,
        }
    }

    #[allow(dead_code)]
    fn new_with_url(url: String) -> Self {
        Self {
            url,
            client: reqwest::Client::new(),
            wait_before_new_prices: ONE_MIN,
        }
    }

    pub async fn value_in_currency(
        &self,
        amount: Amount,
        currency: FiatCurrency,
    ) -> Result<f64, reqwest::Error> {
        let btc = amount.as_btc();
        let price = self.get_price_for(currency).await?;
        let value_in_currency = btc * price as f64;

        Ok(value_in_currency)
    }

    pub async fn get_prices(&self) -> Result<PriceResponse, reqwest::Error> {
        if let Some(prices) = PRICES.load().as_ref() {
            let now_secs = Timestamp::now().as_second() as u64;
            if now_secs - prices.time < self.wait_before_new_prices {
                return Ok(*prices);
            }
        }

        let response = self.client.get(&self.url).send().await?;
        let prices: PriceResponse = response.json().await?;

        // update global prices
        if let Err(error) = update_prices(prices) {
            error!("unable to update prices: {error:?}");
        }

        Ok(prices)
    }

    async fn get_price_for(&self, currency: FiatCurrency) -> Result<u64, reqwest::Error> {
        let prices = self.get_prices().await?;

        let price = match currency {
            FiatCurrency::Usd => prices.usd,
            FiatCurrency::Eur => prices.eur,
            FiatCurrency::Gbp => prices.gbp,
            FiatCurrency::Cad => prices.cad,
            FiatCurrency::Chf => prices.chf,
            FiatCurrency::Aud => prices.aud,
            FiatCurrency::Jpy => prices.jpy,
        };

        Ok(price)
    }
}

/// Get prices from the server, and save them in the database and cache in memory
pub async fn init_prices() -> Result<()> {
    let fiat_client = &FIAT_CLIENT;

    let prices = tryhard::retry_fn(|| fiat_client.get_prices())
        .retries(20)
        .exponential_backoff(Duration::from_millis(10))
        .max_delay(Duration::from_secs(5))
        .await;

    match prices {
        Ok(prices) => {
            PRICES.swap(Arc::new(Some(prices)));

            let db = Database::global();
            db.global_cache
                .set_prices(prices)
                .context("unable to set prices")?;
        }

        Err(error) => {
            warn!("Unable to get prices: {error:?}, using last known prices");
            let db = Database::global();

            if let Some(prices) = db.global_cache.get_prices()? {
                PRICES.swap(Arc::new(Some(prices)));
            }
        }
    }

    Ok(())
}

/// update price in database and cache
fn update_prices(prices: PriceResponse) -> Result<()> {
    PRICES.swap(Arc::new(Some(prices)));

    let db = Database::global();
    db.global_cache
        .set_prices(prices)
        .context("unable to save prices to the database")?;

    Ok(())
}

/// Update prices if needed
pub async fn update_prices_if_needed() -> Result<()> {
    if let Some(prices) = PRICES.load().as_ref() {
        let now_secs = Timestamp::now().as_second() as u64;
        if now_secs - prices.time < ONE_MIN {
            return Ok(());
        }
    }

    let fiat_client = &FIAT_CLIENT;
    let prices = tryhard::retry_fn(|| fiat_client.get_prices())
        .retries(5)
        .exponential_backoff(Duration::from_millis(10))
        .max_delay(Duration::from_secs(1))
        .await?;

    update_prices(prices)?;

    Ok(())
}

mod ffi {
    use tracing::error;

    #[uniffi::export]
    async fn update_prices_if_needed() {
        if let Err(error) = crate::fiat::client::update_prices_if_needed().await {
            error!("unable to update prices: {error:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transaction::Amount;

    #[tokio::test]
    async fn run_all_tests() {
        test_get_prices().await;
        test_get_price_for().await;
        test_get_value_in_usd().await;
        test_get_value_in_usd_with_currency().await;
    }

    async fn test_get_prices() {
        crate::database::delete_database();
        let fiat_client = &FIAT_CLIENT;
        let fiat = fiat_client.get_prices().await.unwrap();
        assert!(fiat.usd > 0);
    }

    async fn test_get_price_for() {
        crate::database::delete_database();
        let fiat_client = &FIAT_CLIENT;
        let fiat = fiat_client.get_price_for(FiatCurrency::Usd).await.unwrap();
        assert!(fiat > 0);
    }

    async fn test_get_value_in_usd() {
        crate::database::delete_database();
        let fiat_client = &FIAT_CLIENT;
        let fiat = fiat_client.get_prices().await.unwrap();
        let value_in_usd = fiat_client
            .value_in_currency(Amount::one_btc(), FiatCurrency::Usd)
            .await
            .unwrap();

        assert_eq!(value_in_usd, fiat.usd as f64);
    }

    async fn test_get_value_in_usd_with_currency() {
        crate::database::delete_database();
        let fiat_client = &FIAT_CLIENT;
        let fiat = fiat_client.get_prices().await.unwrap();

        let half_a_btc = Amount::from_sat(50_000_000);
        let value_in_usd = fiat_client
            .value_in_currency(half_a_btc, FiatCurrency::Usd)
            .await
            .unwrap();

        assert_eq!(value_in_usd, (fiat.usd as f64) / 2.0);
    }
}

#[uniffi::export]
fn prices_are_equal(lhs: Arc<PriceResponse>, rhs: Arc<PriceResponse>) -> bool {
    lhs == rhs
}
