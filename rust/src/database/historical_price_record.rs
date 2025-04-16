use serde::{Deserialize, Serialize};

use crate::fiat::{historical::HistoricalPrice, FiatCurrency};

/// A space-efficient version of HistoricalPrice where only USD is required
/// and other currencies are optional to save space when they aren't available
#[derive(Debug, Copy, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
pub struct HistoricalPriceRecord {
    pub time: u64,
    pub usd: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eur: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gbp: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cad: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chf: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aud: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jpy: Option<f32>,
}

impl From<HistoricalPrice> for HistoricalPriceRecord {
    fn from(price: HistoricalPrice) -> Self {
        Self {
            time: price.time,
            usd: price.usd,
            eur: if price.eur >= 0.0 { Some(price.eur) } else { None },
            gbp: if price.gbp >= 0.0 { Some(price.gbp) } else { None },
            cad: if price.cad >= 0.0 { Some(price.cad) } else { None },
            chf: if price.chf >= 0.0 { Some(price.chf) } else { None },
            aud: if price.aud >= 0.0 { Some(price.aud) } else { None },
            jpy: if price.jpy >= 0.0 { Some(price.jpy) } else { None },
        }
    }
}

#[uniffi::export]
impl HistoricalPriceRecord {
    /// Get the price for a specific currency
    pub fn for_currency(&self, currency: FiatCurrency) -> Option<f32> {
        match currency {
            FiatCurrency::Usd => Some(self.usd),
            FiatCurrency::Eur => self.eur,
            FiatCurrency::Gbp => self.gbp,
            FiatCurrency::Cad => self.cad,
            FiatCurrency::Chf => self.chf,
            FiatCurrency::Aud => self.aud,
            FiatCurrency::Jpy => self.jpy,
        }
    }
}