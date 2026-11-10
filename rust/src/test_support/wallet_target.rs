//! A test's wallet target, wallet lists and wallet selections, restored when the test ends

use cove_device::keychain::Keychain;
use strum::IntoEnumIterator as _;

use crate::{
    database::{Database, global_config::GlobalConfigKey},
    network::Network,
    wallet::metadata::{WalletMetadata, WalletMode},
};

/// Wallet selections a test can change: the live one and the ones saved for each mode
const SELECTION_KEYS: [GlobalConfigKey; 3] = [
    GlobalConfigKey::SelectedWalletId,
    GlobalConfigKey::MainSelectedWalletId,
    GlobalConfigKey::DecoySelectedWalletId,
];

/// Selects a network and wallet mode for one test, then restores them, every wallet list, and
/// every wallet selection
///
/// The target scope starts empty, because other tests in this binary leave wallets behind and a
/// leftover wallet with the same fingerprint would turn an import into a duplicate. Wallets the
/// test creates in any scope are removed with their keychain entries on drop, so a later test
/// that imports the same secret sees a new wallet instead of a duplicate
///
/// Hold it after the global-state lock and before anything that keeps a wallet open, so it
/// restores even when the test panics and before the lock is released
pub(crate) struct WalletTargetGuard {
    network: Network,
    mode: WalletMode,
    selections: Vec<(GlobalConfigKey, Option<String>)>,
    wallets: Vec<(Network, WalletMode, Vec<WalletMetadata>)>,
}

impl WalletTargetGuard {
    pub(crate) fn new(network: Network, mode: WalletMode) -> Self {
        crate::test_support::ensure_tokio_runtime();

        let database = Database::global();
        let config = &database.global_config;
        let guard = Self {
            network: config.selected_network(),
            mode: config.wallet_mode(),
            selections: SELECTION_KEYS.iter().map(|&key| (key, config.get(key).unwrap())).collect(),
            wallets: every_scope()
                .map(|(network, mode)| (network, mode, wallets_in(network, mode)))
                .collect(),
        };

        select_target(network, mode);
        database.wallets.save_all_wallets(network, mode, Vec::new()).unwrap();

        guard
    }
}

impl Drop for WalletTargetGuard {
    fn drop(&mut self) {
        let database = Database::global();
        for (network, mode, original) in &self.wallets {
            for wallet in wallets_in(*network, *mode) {
                if !original.iter().any(|kept| kept.id == wallet.id) {
                    Keychain::global().delete_wallet_items(&wallet.id);
                }
            }

            database.wallets.save_all_wallets(*network, *mode, original.clone()).unwrap();
        }

        // switching modes moves the live selection into the saved ones, so restore the mode first
        select_target(self.network, self.mode);
        for (key, value) in &self.selections {
            match value {
                Some(value) => database.global_config.set(*key, value.clone()).unwrap(),
                None => database.global_config.delete(*key).unwrap(),
            }
        }
    }
}

pub(crate) fn select_target(network: Network, mode: WalletMode) {
    let config = &Database::global().global_config;
    config.set_selected_network(network).unwrap();
    match mode {
        WalletMode::Main => config.set_main_mode().unwrap(),
        WalletMode::Decoy => config.set_decoy_mode().unwrap(),
    }
}

pub(crate) fn every_scope() -> impl Iterator<Item = (Network, WalletMode)> {
    Network::iter().flat_map(|network| [(network, WalletMode::Main), (network, WalletMode::Decoy)])
}

pub(crate) fn wallets_in(network: Network, mode: WalletMode) -> Vec<WalletMetadata> {
    Database::global().wallets.get_all(network, mode).unwrap_or_default()
}
