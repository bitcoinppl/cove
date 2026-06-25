use std::sync::Arc;

use cove_util::format::NumberFormatter as _;

use crate::{
    database::Database,
    fiat::{
        FiatCurrency,
        client::{FIAT_CLIENT, PriceResponse},
    },
    transaction::{Amount, SentAndReceived, TransactionDirection},
    wallet::amount_display,
};

use super::{Error, RustWalletManager};

#[uniffi::export(async_runtime = "tokio")]
impl RustWalletManager {
    #[uniffi::method]
    pub fn selected_fiat_currency(&self) -> FiatCurrency {
        Database::global().global_config.fiat_currency().unwrap_or_default()
    }

    /// Sync method using cached prices, returns None if no cached prices
    #[uniffi::method]
    pub fn amount_in_fiat(&self, amount: Arc<Amount>) -> Option<f64> {
        amount_display::wallet_amount_in_fiat_cached(amount)
    }

    /// Formats a raw amount for display (e.g., "0.00050000 BTC")
    ///
    /// Use this for absolute amounts like balances or input values.
    /// Does NOT include direction prefix - use `display_sent_and_received_amount`
    /// for transaction amounts that need +/- indicators.
    #[uniffi::method(default(show_unit = true))]
    pub fn display_amount(&self, amount: Arc<Amount>, show_unit: bool) -> String {
        let metadata = self.metadata.read().clone();
        amount_display::wallet_display_amount(metadata, amount, show_unit)
    }

    /// Formats a pending BTC amount (e.g. "+ 0.00050000 BTC pending")
    /// Returns None if the amount is zero.
    #[uniffi::method]
    pub fn display_amount_pending_fmt(&self, amount: Arc<Amount>) -> Option<String> {
        let metadata = self.metadata.read().clone();
        amount_display::wallet_display_amount_pending_fmt(metadata, amount)
    }

    /// Formats a BTC amount with direction prefix (e.g., "-0.00050000 BTC")
    ///
    /// Includes "-" prefix for outgoing transactions, no prefix for incoming.
    /// Use this for displaying unsigned transaction BTC amounts in lists.
    #[uniffi::method]
    pub fn display_amount_with_direction(
        &self,
        amount: Arc<Amount>,
        direction: TransactionDirection,
    ) -> String {
        let metadata = self.metadata.read().clone();
        amount_display::wallet_display_amount_with_direction(metadata, amount, direction)
    }

    /// Formats a transaction amount with direction prefix (e.g., "-0.00050000 BTC")
    ///
    /// Includes "-" prefix for outgoing transactions, no prefix for incoming.
    /// Use this for displaying confirmed/unconfirmed transaction amounts in lists.
    #[uniffi::method]
    pub fn display_sent_and_received_amount(
        &self,
        sent_and_received: Arc<SentAndReceived>,
    ) -> String {
        let metadata = self.metadata.read().clone();
        amount_display::wallet_display_sent_and_received_amount(metadata, sent_and_received)
    }

    #[uniffi::method(default(with_suffix = true))]
    pub fn display_fiat_amount(&self, amount: f64, with_suffix: bool) -> String {
        let metadata = self.metadata.read().clone();
        amount_display::wallet_display_fiat_amount(metadata, amount, with_suffix)
    }

    /// Formats a pending fiat amount (e.g. "+ $50.00 pending")
    /// Returns None if the amount is zero.
    #[uniffi::method(default(with_suffix = true))]
    pub fn display_fiat_amount_pending_fmt(
        &self,
        amount: f64,
        with_suffix: bool,
    ) -> Option<String> {
        let metadata = self.metadata.read().clone();
        amount_display::wallet_display_fiat_amount_pending_fmt(metadata, amount, with_suffix)
    }

    /// Formats a fiat amount with direction prefix (e.g., "-$50.00")
    ///
    /// Includes "-" prefix for outgoing transactions, no prefix for incoming.
    /// Use this for displaying confirmed/unconfirmed transaction fiat amounts in lists.
    #[uniffi::method(default(with_suffix = true))]
    pub fn display_fiat_amount_with_direction(
        &self,
        amount: f64,
        direction: TransactionDirection,
        with_suffix: bool,
    ) -> String {
        let metadata = self.metadata.read().clone();
        amount_display::wallet_display_fiat_amount_with_direction(
            metadata,
            amount,
            direction,
            with_suffix,
        )
    }

    #[uniffi::method]
    pub fn convert_to_fiat(&self, amount: Arc<Amount>, prices: Arc<PriceResponse>) -> f64 {
        let currency = self.selected_fiat_currency();
        let price = prices.get_for_currency(currency) as f64;
        ((amount.as_btc() * price) * 100.0).ceil() / 100.0
    }

    #[uniffi::method(default(with_suffix = true))]
    pub fn convert_and_display_fiat(
        &self,
        amount: Arc<Amount>,
        prices: Arc<PriceResponse>,
        with_suffix: bool,
    ) -> String {
        let fiat = self.convert_to_fiat(amount, prices);
        self.display_fiat_amount(fiat, with_suffix)
    }

    #[uniffi::method]
    pub async fn sent_and_received_fiat(
        &self,
        sent_and_received: Arc<SentAndReceived>,
    ) -> Result<f64, Error> {
        let amount = sent_and_received.amount();
        let currency = self.selected_fiat_currency();

        let fiat =
            FIAT_CLIENT.current_value_in_currency(amount, currency).await.map_err(|error| {
                Error::FiatError(format!("unable to get fiat value for amount: {error}"))
            })?;

        Ok(fiat)
    }

    #[uniffi::method]
    pub async fn number_of_confirmations(&self, block_height: u32) -> Result<u32, Error> {
        // always get fresh height to ensure confirmation count reflects latest blocks
        let current_height = self.force_update_height().await?;
        if block_height > current_height { Ok(0) } else { Ok(current_height - block_height + 1) }
    }

    #[uniffi::method]
    pub async fn number_of_confirmations_fmt(&self, block_height: u32) -> Result<String, Error> {
        let number_of_confirmations = self.number_of_confirmations(block_height).await?;
        Ok(number_of_confirmations.thousands_int())
    }
}
