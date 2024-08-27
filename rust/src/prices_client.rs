use std::sync::Arc;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::{impl_default_for, transaction::Amount};
const CURRENCY_URL: &str = "https://mempool.space/api/v1/prices";

#[derive(Debug, Clone, uniffi::Object)]
pub struct PricesClient {
    url: String,
    client: reqwest::Client,
    last_prices: Arc<RwLock<Option<PriceResponse>>>,
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

#[derive(Debug, Copy, Clone, Serialize, Deserialize, uniffi::Enum)]
pub enum Currency {
    Usd,
    Eur,
    Gbp,
    Cad,
    Chf,
    Aud,
    Jpy,
}

impl_default_for!(PricesClient);

impl PricesClient {
    pub fn new() -> Self {
        Self {
            url: CURRENCY_URL.to_string(),
            client: reqwest::Client::new(),
            last_prices: RwLock::new(None).into(),
        }
    }

    pub fn new_with_url(url: String) -> Self {
        Self {
            url,
            client: reqwest::Client::new(),
            last_prices: RwLock::new(None).into(),
        }
    }

    pub async fn value_in_usd(&self, amount: Amount) -> Result<f64, reqwest::Error> {
        self.value_in_currency(amount, Currency::Usd).await
    }

    pub async fn value_in_currency(
        &self,
        amount: Amount,
        currency: Currency,
    ) -> Result<f64, reqwest::Error> {
        let btc = amount.as_btc();
        let price = self.get_price_for(currency).await?;
        let value_in_currency = btc * price as f64;

        Ok(value_in_currency)
    }

    async fn get_price_for(&self, currency: Currency) -> Result<u64, reqwest::Error> {
        let prices = self.get_prices().await?;

        let price = match currency {
            Currency::Usd => prices.usd,
            Currency::Eur => prices.eur,
            Currency::Gbp => prices.gbp,
            Currency::Cad => prices.cad,
            Currency::Chf => prices.chf,
            Currency::Aud => prices.aud,
            Currency::Jpy => prices.jpy,
        };

        Ok(price)
    }

    async fn get_prices(&self) -> Result<PriceResponse, reqwest::Error> {
        if let Some(prices) = self.last_prices.read().await.as_ref() {
            let now_secs = Timestamp::now().as_second() as u64;
            if now_secs - prices.time < 60 {
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
        let prices_client = PricesClient::new();
        let prices = prices_client.get_prices().await.unwrap();
        assert!(prices.usd > 0);
    }

    #[tokio::test]
    async fn test_get_price_for() {
        let prices_client = PricesClient::new();
        let price = prices_client.get_price_for(Currency::Usd).await.unwrap();
        assert!(price > 0);
    }

    #[tokio::test]
    async fn test_get_value_in_usd() {
        let prices_client = PricesClient::new();

        let prices = prices_client.get_prices().await.unwrap();
        let value_in_usd = prices_client.value_in_usd(Amount::one_btc()).await.unwrap();

        let value_in_usd = value_in_usd as f64;
        assert_eq!(value_in_usd, prices.usd as f64);
    }

    #[tokio::test]
    async fn test_get_value_in_usd_with_currency() {
        let prices_client = PricesClient::new();

        let prices = prices_client.get_prices().await.unwrap();

        let half_a_btc = Amount::from_sat(50_000_000);
        let value_in_usd = prices_client.value_in_usd(half_a_btc).await.unwrap();

        let value_in_usd = value_in_usd as f64;
        assert_eq!(value_in_usd, (prices.usd as f64) / 2.0);
    }
}
