use std::sync::{Arc, LazyLock};

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::{fiat::FiatCurrency, transaction::Amount};
use macros::impl_default_for;

const CURRENCY_URL: &str = "https://mempool.space/api/v1/prices";

const FIVE_MINS: u64 = 300;

// Global client for getting prices
pub static FIAT_CLIENT: LazyLock<FiatClient> = LazyLock::new(FiatClient::new);

#[derive(Debug, Clone, uniffi::Object)]
pub struct FiatClient {
    url: String,
    client: reqwest::Client,
    last_prices: Arc<RwLock<Option<PriceResponse>>>,
    wait_before_new_prices: u64,
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
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

impl_default_for!(FiatClient);

impl FiatClient {
    pub fn new() -> Self {
        Self {
            url: CURRENCY_URL.to_string(),
            client: reqwest::Client::new(),
            last_prices: RwLock::new(None).into(),
            wait_before_new_prices: FIVE_MINS,
        }
    }

    pub fn new_with_url(url: String) -> Self {
        Self {
            url,
            client: reqwest::Client::new(),
            last_prices: RwLock::new(None).into(),
            wait_before_new_prices: FIVE_MINS,
        }
    }

    pub async fn value_in_usd(&self, amount: Amount) -> Result<f64, reqwest::Error> {
        self.value_in_currency(amount, FiatCurrency::Usd).await
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

    async fn get_prices(&self) -> Result<PriceResponse, reqwest::Error> {
        if let Some(prices) = self.last_prices.read().await.as_ref() {
            let now_secs = Timestamp::now().as_second() as u64;
            if now_secs - prices.time < self.wait_before_new_prices {
                return Ok(*prices);
            }
        }

        let response = self.client.get(&self.url).send().await?;
        let prices: PriceResponse = response.json().await?;

        let mut prices_guard = self.last_prices.write().await;
        *prices_guard = Some(prices);

        Ok(prices)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transaction::Amount;

    #[tokio::test]
    async fn test_get_prices() {
        let fiat_client = FiatClient::new();
        let fiat = fiat_client.get_prices().await.unwrap();
        assert!(fiat.usd > 0);
    }

    #[tokio::test]
    async fn test_get_price_for() {
        let fiat_client = FiatClient::new();
        let fiat = fiat_client.get_price_for(FiatCurrency::Usd).await.unwrap();
        assert!(fiat > 0);
    }

    #[tokio::test]
    async fn test_get_value_in_usd() {
        let fiat_client = FiatClient::new();

        let fiat = fiat_client.get_prices().await.unwrap();
        let value_in_usd = fiat_client.value_in_usd(Amount::one_btc()).await.unwrap();

        let value_in_usd = value_in_usd as f64;
        assert_eq!(value_in_usd, fiat.usd as f64);
    }

    #[tokio::test]
    async fn test_get_value_in_usd_with_currency() {
        let fiat_client = FiatClient::new();

        let fiat = fiat_client.get_prices().await.unwrap();

        let half_a_btc = Amount::from_sat(50_000_000);
        let value_in_usd = fiat_client.value_in_usd(half_a_btc).await.unwrap();

        let value_in_usd = value_in_usd as f64;
        assert_eq!(value_in_usd, (fiat.usd as f64) / 2.0);
    }
}
