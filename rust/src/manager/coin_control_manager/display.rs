use std::sync::Arc;

use cove_types::amount::Amount;

use crate::{
    fiat::{FiatCurrency, client::PriceResponse},
    wallet::amount_display,
};

use super::RustCoinControlManager;

#[uniffi::export]
impl RustCoinControlManager {
    /// Formats a UTXO amount followed by its fiat value in brackets (e.g. "50,000 SATS ($31.25)")
    ///
    /// Falls back to the bitcoin amount on its own when no prices are available, so the
    /// amount is never followed by empty brackets.
    #[uniffi::method]
    pub fn display_amount_with_fiat(
        &self,
        amount: Arc<Amount>,
        prices: Option<Arc<PriceResponse>>,
        currency: FiatCurrency,
    ) -> String {
        let unit = self.state.lock().unit;
        let bitcoin = amount.fmt_string_with_unit(unit);

        let Some(prices) = prices else { return bitcoin };

        let fiat = amount_display::convert_amount_to_fiat(&amount, &prices, currency);
        let fiat = amount_display::fmt_fiat_amount(currency, fiat, true);

        format!("{bitcoin} ({fiat})")
    }
}

#[cfg(test)]
mod tests {
    use cove_types::unit::BitcoinUnit;

    use super::*;

    /// 0.0005 BTC (50,000 sats) is worth 31.25 at this price
    fn prices() -> Arc<PriceResponse> {
        Arc::new(PriceResponse {
            time: 0,
            fetched_at: 0,
            usd: 62_500,
            eur: 62_500,
            gbp: 62_500,
            cad: 62_500,
            chf: 62_500,
            aud: 62_500,
            jpy: 62_500,
        })
    }

    fn manager(unit: BitcoinUnit) -> RustCoinControlManager {
        let manager = RustCoinControlManager::preview_new(1, 0);
        manager.state.lock().unit = unit;

        manager
    }

    #[test]
    fn display_amount_with_fiat_appends_fiat_value_in_brackets() {
        let manager = manager(BitcoinUnit::Sat);
        let amount = Arc::new(Amount::from_sat(50_000));

        assert_eq!(
            manager.display_amount_with_fiat(amount, Some(prices()), FiatCurrency::Usd),
            "50,000 SATS ($31.25)"
        );
    }

    #[test]
    fn display_amount_with_fiat_uses_the_selected_unit_and_currency() {
        let manager = manager(BitcoinUnit::Btc);
        let amount = Arc::new(Amount::from_sat(50_000));

        assert_eq!(
            manager.display_amount_with_fiat(amount, Some(prices()), FiatCurrency::Chf),
            "0.0005 BTC (31.25 CHF)"
        );
    }

    #[test]
    fn display_amount_with_fiat_omits_brackets_without_prices() {
        let manager = manager(BitcoinUnit::Sat);
        let amount = Arc::new(Amount::from_sat(50_000));

        assert_eq!(
            manager.display_amount_with_fiat(amount, None, FiatCurrency::Usd),
            "50,000 SATS"
        );
    }
}
