//! Fixtures shared by the end-to-end KeyTeleport transfer tests

use bdk_wallet::{
    KeychainKind,
    bitcoin::{NetworkKind, bip32::Xpriv, secp256k1::Secp256k1},
    template::{Bip84, DescriptorTemplate as _},
};
use strum::IntoEnumIterator as _;

use crate::{multi_format::StringOrData, wallet::Wallet, wallet::fingerprint::Fingerprint};

use super::*;

/// The published BIP84 test vector mnemonic
pub(super) const ABANDON_ABOUT: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

/// Selects a network and wallet mode for one test, then restores them and every wallet list
///
/// Wallets the test creates in any scope are removed with their keychain entries on drop, so a
/// later test that imports the same secret sees a new wallet instead of a duplicate
pub(super) struct WalletTargetGuard {
    network: Network,
    mode: WalletMode,
    wallets: Vec<(Network, WalletMode, Vec<WalletMetadata>)>,
}

impl WalletTargetGuard {
    pub(super) fn new(network: Network, mode: WalletMode) -> Self {
        crate::test_support::ensure_tokio_runtime();

        let config = &Database::global().global_config;
        let guard = Self {
            network: config.selected_network(),
            mode: config.wallet_mode(),
            wallets: every_scope()
                .map(|(network, mode)| (network, mode, wallets_in(network, mode)))
                .collect(),
        };

        select_target(network, mode);
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

        select_target(self.network, self.mode);
    }
}

pub(super) fn select_target(network: Network, mode: WalletMode) {
    let config = &Database::global().global_config;
    config.set_selected_network(network).unwrap();
    match mode {
        WalletMode::Main => config.set_main_mode().unwrap(),
        WalletMode::Decoy => config.set_decoy_mode().unwrap(),
    }
}

pub(super) fn every_scope() -> impl Iterator<Item = (Network, WalletMode)> {
    Network::iter().flat_map(|network| [(network, WalletMode::Main), (network, WalletMode::Decoy)])
}

pub(super) fn wallets_in(network: Network, mode: WalletMode) -> Vec<WalletMetadata> {
    Database::global().wallets.get_all(network, mode).unwrap_or_default()
}

/// Every wallet on the device, across all networks and modes
pub(super) fn all_wallet_ids() -> Vec<WalletId> {
    every_scope()
        .flat_map(|(network, mode)| wallets_in(network, mode))
        .map(|wallet| wallet.id)
        .collect()
}

/// Opens Receive on a new manager, as the receiving phone does, and returns its request
pub(super) fn start_receiver() -> (Arc<RustKeyTeleportManager>, KeyTeleportReceiveState) {
    let manager = RustKeyTeleportManager::new();
    manager.handle_action(Action::StartReceive).unwrap();
    let request = receive_request(&manager);

    (manager, request)
}

/// Scans the sender's response and types its password on the receiving phone
pub(super) fn deliver_response(
    manager: &RustKeyTeleportManager,
    packet: &KeyTeleportSenderPacket,
    password: &str,
) -> Result<(), KeyTeleportAlert> {
    let scanned = StringOrData::String(packet.bbqr_part().unwrap());
    manager.handle_action(Action::Ingest(KeyTeleportInput::MultiFormat(scanned)))?;
    manager.handle_action(Action::EnterSenderPassword(password.to_string()))
}

/// Imports the reviewed secret and returns the new wallet, failing on any other outcome
pub(super) fn import_new_wallet(manager: &RustKeyTeleportManager) -> WalletMetadata {
    manager.handle_action(Action::ImportReceivedWallet).unwrap();
    let KeyTeleportManagerState::ReceiveImportedWallet(wallet) = manager.state() else {
        panic!("expected a newly imported wallet, got {:?}", manager.state())
    };

    wallet
}

/// Sends `secret` through the send flow a Cove phone runs, then removes the sending wallet
///
/// Both phones share one test database, so the sender's wallet is removed before the receiver
/// imports, the way a second phone has never seen it
pub(super) fn send_from_cove(
    secret: WalletSecret,
    request: &KeyTeleportReceiveState,
) -> (Arc<KeyTeleportSenderPacket>, String) {
    let fixture = SendWalletFixture::with_secret(secret);
    let sender = RustKeyTeleportManager::new();
    let scanned_request = StringOrData::String(request.packet.url().unwrap());

    sender.handle_action(Action::StartSendFromWallet(fixture.wallet.id.clone())).unwrap();
    sender.handle_action(Action::Ingest(KeyTeleportInput::MultiFormat(scanned_request))).unwrap();
    sender.handle_action(Action::EnterReceiverCode(request.grouped_numeric_code.clone())).unwrap();

    let KeyTeleportManagerState::SendReady(ready) = sender.state() else {
        panic!("expected the sender response, got {:?}", sender.state())
    };

    (ready.packet, ready.password.display_text())
}

/// The wallet a receiver should end up with, derived with BDK's BIP84 template instead of Cove's
/// own descriptor code
pub(super) struct ExpectedWallet {
    pub(super) network: Network,
    pub(super) fingerprint: Fingerprint,
    pub(super) first_address: String,
}

impl ExpectedWallet {
    pub(super) fn from_mnemonic(mnemonic: &Mnemonic, network: Network) -> Self {
        Self::from_seed(&mnemonic.to_seed(""), network)
    }

    pub(super) fn from_seed(seed: &[u8], network: Network) -> Self {
        Self::from_xprv(Xpriv::new_master(NetworkKind::Main, seed).unwrap(), network)
    }

    pub(super) fn from_xprv(mut xprv: Xpriv, network: Network) -> Self {
        let bitcoin_network = bdk_wallet::bitcoin::Network::from(network);
        xprv.network = bitcoin_network.into();

        let (descriptor, _, _) = Bip84(xprv, KeychainKind::External).build(xprv.network).unwrap();
        let first_address = descriptor
            .at_derivation_index(0)
            .unwrap()
            .address(bitcoin_network)
            .unwrap()
            .to_string();

        Self { network, fingerprint: xprv.fingerprint(&Secp256k1::new()).into(), first_address }
    }

    /// Checks the wallet the receiver imported the way its Receive screen and wallet list see it
    pub(super) fn assert_imported(&self, wallet: &WalletMetadata) {
        assert_eq!(wallet.network, self.network, "imported wallet network");
        assert_eq!(wallet.master_fingerprint.as_deref(), Some(&self.fingerprint));
        assert_eq!(wallet.wallet_type, WalletType::Hot);

        let loaded = Wallet::try_load_persisted(wallet.id.clone()).unwrap();
        let first_address = loaded.bdk.peek_address(KeychainKind::External, 0).address;
        assert_eq!(loaded.network, self.network, "persisted wallet network");
        assert_eq!(first_address.to_string(), self.first_address, "first receive address");
    }
}
