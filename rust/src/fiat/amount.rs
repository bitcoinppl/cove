use crate::transaction::SentAndReceived;

use super::{FiatCurrency, client::PRICES};

#[derive(Debug, thiserror::Error, derive_more::Display, uniffi::Error)]
pub enum FiatAmountError {
    /// Unable to convert to fiat amount, prices client unavailable {0}
    PricesUnavailable(String),
}

type Result<T, E = FiatAmountError> = std::result::Result<T, E>;

#[derive(Debug, Clone, Copy, uniffi::Record)]
pub struct FiatAmount {
    pub amount: f64,
    pub currency: FiatCurrency,
}

impl FiatAmount {
    pub fn try_new(sent_and_received: &SentAndReceived, currency: FiatCurrency) -> Result<Self> {
        let prices = PRICES.load().as_ref().ok_or_else(|| {
            crate::task::spawn(async {
                let _ = crate::fiat::client::fetch_and_update_prices_if_needed().await;
            });

            FiatAmountError::PricesUnavailable("prices not available".to_string())
        })?;

        let amount = sent_and_received.amount();
        let fiat = amount.as_btc() * prices.get_for_currency(currency) as f64;

        Ok(Self { amount: fiat, currency })
    }
}

// PREVIEW ONLY
//
impl FiatAmount {
    pub const fn preview_new() -> Self {
        Self { amount: 120.38, currency: FiatCurrency::Usd }
    }
}

#[uniffi::export]
fn fiat_amount_preview_new() -> FiatAmount {
    FiatAmount::preview_new()
}

impl Eq for FiatAmount {}
impl PartialEq for FiatAmount {
    fn eq(&self, other: &Self) -> bool {
        self.amount.ceil() == other.amount.ceil() && self.currency == other.currency
    }
}
