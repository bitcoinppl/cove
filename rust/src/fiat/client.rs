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

use super::historical::HistoricalPricesResponse;

const CURRENCY_URL: &str = "https://mempool.space/api/v1/prices";
const HISTORICAL_PRICES_URL: &str = "https://mempool.space/api/v1/historical-price";

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
#[serde(rename_all = "UPPERCASE")]
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
    
    /// Fetch and store historical price for a given block number
    /// This combines the API call with database storage in one method
    pub async fn fetch_and_store_price_for_block(&self, block_number: u32, timestamp: u64) -> Result<()> {
        let historical_data = self.historical_prices(timestamp).await
            .map_err(|e| eyre::eyre!("Failed to fetch historical price: {}", e))?;
            
        if historical_data.prices.is_empty() {
            return Err(eyre::eyre!("No price data available for timestamp {}", timestamp));
        }
        
        // Store the first price entry in the database (converted to the space-efficient record format)
        let price_record = super::historical::HistoricalPriceRecord::from(historical_data.prices[0]);
        store_historical_price_record_for_block(block_number, price_record)
            .context("Failed to store historical price for block")
    }

    #[allow(dead_code)]
    fn new_with_url(url: String) -> Self {
        Self {
            url,
            client: reqwest::Client::new(),
            wait_before_new_prices: ONE_MIN,
        }
    }

    /// Get historical price and exchange rates for a specific timestamp
    /// - timestamp: Unix timestamp in seconds for which to get the price
    pub async fn historical_prices(
        &self,
        timestamp: u64,
    ) -> Result<HistoricalPricesResponse, reqwest::Error> {
        let url = format!("{}?timestamp={}", HISTORICAL_PRICES_URL, timestamp);

        let response = self.client.get(&url).send().await?;
        let historical_prices: HistoricalPricesResponse = response.json().await?;

        Ok(historical_prices)
    }

    /// Get historical price for Bitcoin in the requested currency at a given timestamp
    pub async fn historical_price_for_currency(
        &self,
        timestamp: u64,
        currency: FiatCurrency,
    ) -> Result<Option<f32>, reqwest::Error> {
        let historical_data = self.historical_prices(timestamp).await?;
        Ok(historical_data.for_currency(currency))
    }

    /// Convert the BTC amount to the requested currency using a historical price
    pub async fn historical_value_in_currency(
        &self,
        amount: Amount,
        currency: FiatCurrency,
        timestamp: u64,
    ) -> Result<Option<f64>, reqwest::Error> {
        let btc = amount.as_btc();
        let price = self
            .historical_price_for_currency(timestamp, currency)
            .await?;

        if price.is_none() {
            return Ok(None);
        }

        let value_in_currency = btc * price.expect("price is some") as f64;
        Ok(Some(value_in_currency))
    }

    /// Get the current price for a currency
    pub async fn prices(&self) -> Result<PriceResponse, reqwest::Error> {
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

    async fn price_for(&self, currency: FiatCurrency) -> Result<u64, reqwest::Error> {
        let prices = self.prices().await?;

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

    /// Convert the BTC amount to the requested currency using the current price
    pub async fn current_value_in_currency(
        &self,
        amount: Amount,
        currency: FiatCurrency,
    ) -> Result<f64, reqwest::Error> {
        let btc = amount.as_btc();
        let price = self.price_for(currency).await?;
        let value_in_currency = btc * price as f64;

        Ok(value_in_currency)
    }
}

/// Get prices from the server, and save them in the database and cache in memory
pub async fn init_prices() -> Result<()> {
    let fiat_client = &FIAT_CLIENT;

    let prices = tryhard::retry_fn(|| fiat_client.prices())
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
    let prices = tryhard::retry_fn(|| fiat_client.prices())
        .retries(5)
        .exponential_backoff(Duration::from_millis(10))
        .max_delay(Duration::from_secs(1))
        .await?;

    update_prices(prices)?;

    Ok(())
}

/// Store historical price for a specific block number
pub fn store_historical_price_for_block(block_number: u32, price: super::historical::HistoricalPrice) -> Result<()> {
    let db = Database::global();
    db.historical_prices.set_price_for_block(block_number, price)
        .context("unable to save historical price")
}

/// Store a space-efficient historical price record for a specific block number
pub fn store_historical_price_record_for_block(block_number: u32, price_record: super::historical::HistoricalPriceRecord) -> Result<()> {
    let db = Database::global();
    db.historical_prices.set_price_record_for_block(block_number, price_record)
        .context("unable to save historical price record")
}

/// Get historical price for a specific block number
pub fn get_historical_price_for_block(block_number: u32) -> Result<Option<super::historical::HistoricalPriceRecord>> {
    let db = Database::global();
    db.historical_prices.get_price_by_block(block_number)
        .context("unable to get historical price")
}

/// Get all historical prices
pub fn get_all_historical_prices() -> Result<Vec<(crate::database::historical_prices::BlockNumber, super::historical::HistoricalPriceRecord)>> {
    let db = Database::global();
    db.historical_prices.get_all_prices()
        .context("unable to get all historical prices")
}

mod ffi {
    use tracing::error;
    use super::super::historical::{HistoricalPrice, HistoricalPriceRecord};
    use crate::database::historical_prices::BlockNumber;

    #[uniffi::export]
    async fn update_prices_if_needed() {
        if let Err(error) = crate::fiat::client::update_prices_if_needed().await {
            error!("unable to update prices: {error:?}");
        }
    }
    
    #[uniffi::export]
    async fn fetch_and_store_price_for_block(block_number: u32, timestamp: u64) -> Result<(), String> {
        match crate::fiat::client::FIAT_CLIENT.fetch_and_store_price_for_block(block_number, timestamp).await {
            Ok(()) => Ok(()),
            Err(error) => Err(format!("Error fetching and storing historical price: {error}")),
        }
    }
    
    #[uniffi::export]
    fn store_historical_price_for_block(block_number: u32, price: HistoricalPrice) -> Result<(), String> {
        match crate::fiat::client::store_historical_price_for_block(block_number, price) {
            Ok(()) => Ok(()),
            Err(error) => Err(format!("Error storing historical price: {error}")),
        }
    }
    
    #[uniffi::export]
    fn store_historical_price_record_for_block(block_number: u32, price_record: HistoricalPriceRecord) -> Result<(), String> {
        match crate::fiat::client::store_historical_price_record_for_block(block_number, price_record) {
            Ok(()) => Ok(()),
            Err(error) => Err(format!("Error storing historical price record: {error}")),
        }
    }
    
    #[uniffi::export]
    fn get_historical_price_for_block(block_number: u32) -> Result<Option<HistoricalPriceRecord>, String> {
        match crate::fiat::client::get_historical_price_for_block(block_number) {
            Ok(price) => Ok(price),
            Err(error) => Err(format!("Error getting historical price: {error}")),
        }
    }
    
    #[uniffi::export]
    fn get_all_historical_prices() -> Result<Vec<(BlockNumber, HistoricalPriceRecord)>, String> {
        match crate::fiat::client::get_all_historical_prices() {
            Ok(prices) => Ok(prices),
            Err(error) => Err(format!("Error getting all historical prices: {error}")),
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
        test_get_historical_prices().await;
        test_historical_price_at_time().await;
    }

    async fn test_get_prices() {
        crate::database::delete_database();
        let fiat_client = &FIAT_CLIENT;
        let fiat = fiat_client.prices().await.unwrap();
        assert!(fiat.usd > 0);
    }

    async fn test_get_price_for() {
        crate::database::delete_database();
        let fiat_client = &FIAT_CLIENT;
        let fiat = fiat_client.price_for(FiatCurrency::Usd).await.unwrap();
        assert!(fiat > 0);
    }

    async fn test_get_value_in_usd() {
        crate::database::delete_database();
        let fiat_client = &FIAT_CLIENT;
        let fiat = fiat_client.prices().await.unwrap();
        let value_in_usd = fiat_client
            .current_value_in_currency(Amount::one_btc(), FiatCurrency::Usd)
            .await
            .unwrap();

        assert_eq!(value_in_usd, fiat.usd as f64);
    }

    async fn test_get_value_in_usd_with_currency() {
        crate::database::delete_database();
        let fiat_client = &FIAT_CLIENT;
        let fiat = fiat_client.prices().await.unwrap();

        let half_a_btc = Amount::from_sat(50_000_000);
        let value_in_usd = fiat_client
            .current_value_in_currency(half_a_btc, FiatCurrency::Usd)
            .await
            .unwrap();

        assert_eq!(value_in_usd, (fiat.usd as f64) / 2.0);
    }

    async fn test_get_historical_prices() {
        let fiat_client = &FIAT_CLIENT;

        // Get historical prices for current timestamp
        let now = Timestamp::now().as_second() as u64;

        let historical_prices = fiat_client.historical_prices(now).await.unwrap();

        // Verify we got some price data
        assert!(!historical_prices.prices.is_empty());

        // Verify the prices have a valid timestamp
        for price in &historical_prices.prices {
            assert!(price.time > 0);
            assert!(price.usd > 0.0);
            assert!(price.eur > 0.0);
            assert!(price.gbp > 0.0);
            assert!(price.cad > 0.0);
            assert!(price.chf > 0.0);
            assert!(price.aud > 0.0);
            assert!(price.jpy > 0.0);
        }
    }

    async fn test_historical_price_at_time() {
        let fiat_client = &FIAT_CLIENT;

        // Use a known timestamp (now - 12 hours)
        let timestamp = Timestamp::now().as_second() as u64 - (12 * 60 * 60);

        // Test for USD
        let price_usd = fiat_client
            .historical_price_for_currency(timestamp, FiatCurrency::Usd)
            .await
            .unwrap();

        assert!(price_usd.is_some());
        let price_usd = price_usd.unwrap();
        assert!(price_usd > 0.0);

        // Test for EUR
        let price_eur = fiat_client
            .historical_price_for_currency(timestamp, FiatCurrency::Eur)
            .await
            .unwrap();

        assert!(price_eur.is_some());
        let price_eur = price_eur.unwrap();
        assert!(price_eur > 0.0);
    }
}

#[uniffi::export]
fn prices_are_equal(lhs: Arc<PriceResponse>, rhs: Arc<PriceResponse>) -> bool {
    lhs == rhs
}
