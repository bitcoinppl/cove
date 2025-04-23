use serde::{Deserialize, Serialize};

use super::FiatCurrency;

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Object)]
#[serde(rename_all = "camelCase")]
pub struct HistoricalPricesResponse {
    pub prices: Vec<HistoricalPrice>,
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "UPPERCASE")]
pub struct HistoricalPrice {
    #[serde(rename = "time")]
    pub time: u64,
    pub usd: f32,
    pub eur: f32,
    pub gbp: f32,
    pub cad: f32,
    pub chf: f32,
    pub aud: f32,
    pub jpy: f32,
}

impl HistoricalPrice {
    pub fn for_currency(&self, currency: FiatCurrency) -> Option<f32> {
        let price = match currency {
            FiatCurrency::Usd => self.usd,
            FiatCurrency::Eur => self.eur,
            FiatCurrency::Gbp => self.gbp,
            FiatCurrency::Cad => self.cad,
            FiatCurrency::Chf => self.chf,
            FiatCurrency::Aud => self.aud,
            FiatCurrency::Jpy => self.jpy,
        };

        // in the mempool.space API, the price is negative if not available
        if price < 0.0 {
            return None;
        }

        Some(price)
    }
}
