use cove_types::unit::BitcoinUnit;

use crate::wallet::metadata::FiatOrBtc;

use super::RustSendFlowManager;

/// Shows amounts in `unit`, typed in bitcoin, applied the way the wallet manager's push applies it
///
/// This is test setup, so it discards the reconcile messages it causes and a test reads only the
/// ones its own steps send
pub(crate) fn show_bitcoin_unit(manager: &RustSendFlowManager, unit: BitcoinUnit) {
    let mut preferences = manager.wallet_manager.metadata.display_preferences();
    preferences.selected_unit = unit;
    preferences.fiat_or_btc = FiatOrBtc::Btc;
    manager.apply_display_preferences(preferences);

    while manager.reconciler.receiver().try_recv().is_ok() {}
}
