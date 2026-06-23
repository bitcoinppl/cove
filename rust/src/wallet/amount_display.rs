use std::sync::Arc;

use cove_util::format::NumberFormatter as _;

use crate::{
    database::Database,
    fiat::{FiatCurrency, client::FIAT_CLIENT},
    transaction::{Amount, SentAndReceived, TransactionDirection, Unit},
    wallet::metadata::WalletMetadata,
};

const BTC_MASK: &str = "••••••";
const FIAT_MASK: &str = "**************";

#[uniffi::export]
pub fn wallet_display_amount(
    metadata: WalletMetadata,
    amount: Arc<Amount>,
    show_unit: bool,
) -> String {
    display_amount(metadata.sensitive_visible, metadata.selected_unit, *amount, show_unit)
}

#[uniffi::export]
pub fn wallet_display_amount_pending_fmt(
    metadata: WalletMetadata,
    amount: Arc<Amount>,
) -> Option<String> {
    if amount.as_sats() == 0 {
        return None;
    }

    let formatted = wallet_display_amount(metadata, amount, true);
    Some(format!("+ {formatted} pending"))
}

pub(crate) fn display_amount(
    sensitive_visible: bool,
    unit: Unit,
    amount: Amount,
    show_unit: bool,
) -> String {
    if !sensitive_visible {
        return BTC_MASK.to_string();
    }

    if show_unit { amount.fmt_string_with_unit(unit) } else { amount.fmt_string(unit) }
}

#[uniffi::export]
pub fn wallet_display_amount_with_direction(
    metadata: WalletMetadata,
    amount: Arc<Amount>,
    direction: TransactionDirection,
) -> String {
    let formatted =
        display_amount(metadata.sensitive_visible, metadata.selected_unit, *amount, true);
    display_with_direction(formatted, direction)
}

pub(crate) fn display_with_direction(formatted: String, direction: TransactionDirection) -> String {
    match direction {
        TransactionDirection::Outgoing => format!("-{formatted}"),
        TransactionDirection::Incoming => formatted,
    }
}

#[uniffi::export]
pub fn wallet_display_sent_and_received_amount(
    metadata: WalletMetadata,
    sent_and_received: Arc<SentAndReceived>,
) -> String {
    display_sent_and_received_amount(
        metadata.sensitive_visible,
        metadata.selected_unit,
        &sent_and_received,
    )
}

pub(crate) fn display_sent_and_received_amount(
    sensitive_visible: bool,
    unit: Unit,
    sent_and_received: &SentAndReceived,
) -> String {
    if !sensitive_visible {
        return BTC_MASK.to_string();
    }

    sent_and_received.amount_fmt(unit)
}

#[uniffi::export]
pub fn wallet_display_fiat_amount(
    metadata: WalletMetadata,
    amount: f64,
    with_suffix: bool,
) -> String {
    let currency = selected_fiat_currency();
    display_fiat_amount_with_currency(metadata.sensitive_visible, currency, amount, with_suffix)
}

#[uniffi::export]
pub fn wallet_display_fiat_amount_pending_fmt(
    metadata: WalletMetadata,
    amount: f64,
    with_suffix: bool,
) -> Option<String> {
    if amount <= 0.0 {
        return None;
    }

    let formatted = wallet_display_fiat_amount(metadata, amount, with_suffix);
    Some(format!("+ {formatted} pending"))
}

#[uniffi::export]
pub fn wallet_display_fiat_amount_with_direction(
    metadata: WalletMetadata,
    amount: f64,
    direction: TransactionDirection,
    with_suffix: bool,
) -> String {
    let prefix = match direction {
        TransactionDirection::Incoming => "",
        TransactionDirection::Outgoing => "-",
    };

    format!("{prefix}{}", wallet_display_fiat_amount(metadata, amount, with_suffix))
}

#[uniffi::export]
pub fn wallet_amount_in_fiat_cached(amount: Arc<Amount>) -> Option<f64> {
    let currency = selected_fiat_currency();
    FIAT_CLIENT.value_in_currency_cached(*amount, currency)
}

fn selected_fiat_currency() -> FiatCurrency {
    Database::global().global_config.fiat_currency().unwrap_or_default()
}

pub(crate) fn display_fiat_amount_with_currency(
    sensitive_visible: bool,
    currency: FiatCurrency,
    amount: f64,
    with_suffix: bool,
) -> String {
    if !sensitive_visible {
        return FIAT_MASK.to_string();
    }

    let fiat = amount.thousands_fiat();
    let symbol = currency.symbol();
    let suffix = currency.suffix();

    if with_suffix && !suffix.is_empty() {
        return format!("{symbol}{fiat} {suffix}");
    }

    format!("{symbol}{fiat}")
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use cove_types::unit::BitcoinUnit;

    use super::*;
    use crate::wallet::metadata::WalletMetadata;

    #[test]
    fn wallet_amount_display_uses_metadata_unit() {
        let mut metadata = WalletMetadata::preview_new();
        let amount = Arc::new(Amount::from_sat(12_000));

        metadata.selected_unit = BitcoinUnit::Sat;
        assert_eq!(wallet_display_amount(metadata.clone(), amount.clone(), true), "12,000 SATS");
        assert_eq!(wallet_display_amount(metadata.clone(), amount.clone(), false), "12,000");

        metadata.selected_unit = BitcoinUnit::Btc;
        assert_eq!(wallet_display_amount(metadata, amount, true), "0.00012 BTC");
    }

    #[test]
    fn wallet_amount_display_masks_hidden_metadata() {
        let mut metadata = WalletMetadata::preview_new();
        metadata.sensitive_visible = false;

        let amount = Arc::new(Amount::from_sat(12_000));
        assert_eq!(wallet_display_amount(metadata, amount, true), BTC_MASK);
    }

    #[test]
    fn wallet_amount_pending_display_omits_zero_amounts() {
        let mut metadata = WalletMetadata::preview_new();
        metadata.selected_unit = BitcoinUnit::Sat;

        assert_eq!(
            wallet_display_amount_pending_fmt(metadata.clone(), Arc::new(Amount::ZERO)),
            None
        );
        assert_eq!(
            wallet_display_amount_pending_fmt(metadata, Arc::new(Amount::from_sat(1))),
            Some("+ 1 SATS pending".to_string())
        );
    }

    #[test]
    fn wallet_fiat_display_uses_currency_suffix_rules() {
        assert_eq!(
            display_fiat_amount_with_currency(true, FiatCurrency::Usd, 20_000.0, true),
            "$20,000.00"
        );
        assert_eq!(
            display_fiat_amount_with_currency(true, FiatCurrency::Cad, 20_000.0, true),
            "$20,000.00 CAD"
        );
        assert_eq!(
            display_fiat_amount_with_currency(true, FiatCurrency::Chf, 20_000.0, true),
            "20,000.00 CHF"
        );
    }
}
