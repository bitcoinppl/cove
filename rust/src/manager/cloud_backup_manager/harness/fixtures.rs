//! Small per-case accounts, plus the lived-in account for the incident replay

use std::sync::Arc;
use std::time::Duration;

use bdk_wallet::bitcoin::bip32::Xpub;

use cove_cspp::Cspp;
use cove_cspp::backup_data::{wallet_filename_from_record_id, wallet_record_id};
use cove_device::keychain::Keychain;
use cove_device::passkey::DiscoveredPasskeyResult;

use super::app::{NextAction, TestApp};
use crate::database::Database;
use crate::database::cloud_backup::PersistedCloudBackupState;
use crate::manager::cloud_backup_manager::ops::test_support::{
    enable_cloud_backup_without_reset, encrypted_wallet_backup_bytes_for_entry,
    seed_verifiable_cloud_master_key, test_globals, wallet_entry_with_labels,
    xpub_only_wallet_metadata,
};
use crate::manager::cloud_backup_manager::owner::ui_test_legacy_record::legacy_stuck_record;
use crate::manager::cloud_backup_manager::{
    CloudBackupKeychain, CloudBackupManagerAction, CloudBackupStore,
};
use crate::mnemonic::{MnemonicExt as _, NumberOfBip39Words};
use crate::network::Network;
use crate::wallet::fingerprint::Fingerprint;
use crate::wallet::metadata::WalletId;
use crate::wallet::metadata::WalletMetadata;

/// The records an account has in its active namespace
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FakeAccount {
    /// Wallets on this device with a backup in the namespace
    pub(crate) local_wallets: usize,
    /// Backups in the namespace for wallets that are not on this device
    pub(crate) not_on_device: usize,
}

impl FakeAccount {
    pub(crate) fn new(local_wallets: usize, not_on_device: usize) -> Self {
        Self { local_wallets, not_on_device }
    }

    /// The incident replay and layout account: 35 records, some only in the cloud
    ///
    /// Only for those tests; concurrency cases use the smallest account that shows the case
    pub(crate) fn lived_in() -> Self {
        Self { local_wallets: 31, not_on_device: 4 }
    }

    pub(crate) fn record_count(&self) -> usize {
        self.local_wallets + self.not_on_device
    }

    /// Configure Cloud Backup on this account and return its namespace
    pub(crate) fn seed(&self, app: &TestApp) -> String {
        self.seed_records(app).namespace
    }

    /// Configure Cloud Backup on this account and return its namespace and records
    pub(crate) fn seed_records(&self, app: &TestApp) -> SeededAccount {
        let wallet_count = u32::try_from(self.local_wallets).expect("wallet count fits u32");
        enable_cloud_backup_without_reset(app.manager(), wallet_count);
        let namespace = CloudBackupKeychain::global()
            .namespace_id()
            .expect("read namespace")
            .expect("seeded namespace");

        let master_key = Cspp::new(Keychain::global().clone())
            .load_master_key_from_store()
            .expect("read master key")
            .expect("seeded master key");

        // real encrypted backups of distinct wallets, so the detail inventory loads and sorts
        // them as it does for a user: each local wallet backs up its own record, and each record
        // not on this device matches no local wallet
        let target = Database::global().global_config.wallet_target();
        let local: Vec<_> =
            (0..self.local_wallets).map(|_| DistinctWallet::new(target.network)).collect();
        let elsewhere: Vec<_> =
            (0..self.not_on_device).map(|_| DistinctWallet::new(target.network)).collect();
        let not_on_device =
            elsewhere.iter().map(|wallet| wallet_record_id(wallet.metadata.id.as_ref())).collect();
        for wallet in &local {
            Keychain::global()
                .save_wallet_xpub(&wallet.metadata.id, wallet.xpub)
                .expect("save local wallet xpub");
        }

        let local_metadata = local.iter().map(|wallet| wallet.metadata.clone()).collect();
        Database::global()
            .wallets()
            .save_all_wallets(target.network, target.mode, local_metadata)
            .expect("save local wallets");

        let mut files = Vec::with_capacity(self.record_count());
        for wallet in local.into_iter().chain(elsewhere) {
            let record_id = wallet_record_id(wallet.metadata.id.as_ref());
            let mut entry = wallet_entry_with_labels(&wallet.metadata, None);
            entry.xpub = Some(wallet.xpub.to_string());
            let backup = encrypted_wallet_backup_bytes_for_entry(&entry, &master_key, 1);
            app.cloud.inner.set_wallet_backup(namespace.clone(), record_id.clone(), backup);
            files.push(wallet_filename_from_record_id(&record_id));
        }

        app.cloud.inner.set_wallet_files(namespace.clone(), files);
        app.cloud.inner.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);
        app.cloud.persist();
        app.settle();

        SeededAccount { namespace, local: local_records, not_on_device }
    }
}

/// What [`FakeAccount::seed_records`] put in the cloud
#[derive(Debug, Clone)]
pub(crate) struct SeededAccount {
    pub(crate) namespace: String,
    /// The backups of the wallets on this device
    pub(crate) local: Vec<SeededRecord>,
    /// Record ids of the backups for wallets that are not on this device
    pub(crate) not_on_device: Vec<String>,
}

/// A watch-only wallet with its own key, so no two seeded wallets share an identity
struct DistinctWallet {
    metadata: WalletMetadata,
    xpub: Xpub,
}

impl DistinctWallet {
    fn new(network: Network) -> Self {
        let xpub = NumberOfBip39Words::Twelve.generate_mnemonic().xpub(network.into());
        let mut metadata = xpub_only_wallet_metadata();
        metadata.network = network;
        metadata.master_fingerprint = Some(Arc::new(Fingerprint::from(xpub.fingerprint())));

        Self { metadata, xpub }
    }
}

/// Replace the seeded account's placeholder master key backup with a real wrapper, and set up
/// the passkey that opens it, so a verification reaches the passkey and can verify
pub(crate) fn seed_verifiable_master_key(app: &TestApp) {
    seed_verifiable_cloud_master_key(test_globals());
    app.cloud.persist();
}

/// Persist the reporter's stuck record: a disable that never deleted anything
pub(crate) fn seed_legacy_stuck_disabling(namespace: &str, started_at: u64) {
    let table = &Database::global().cloud_backup_state;
    let PersistedCloudBackupState::Configured(previous_configured) =
        table.get().expect("read cloud backup state")
    else {
        panic!("the legacy record replaces a configured record");
    };

    table
        .set(&legacy_stuck_record(previous_configured, namespace.to_string(), started_at))
        .expect("persist legacy disabling record");
}

/// Persist a lifecycle record that no longer decodes, as a damaged database leaves it
pub(crate) fn seed_corrupted_record(error: &str) {
    Database::global()
        .cloud_backup_state
        .set(&PersistedCloudBackupState::corrupted(error))
        .expect("persist corrupted record");
}

/// Changes the first local wallet's backed-up metadata the way the wallet screens report it, and
/// returns it, or `None` when the device has no wallet
pub(crate) fn change_a_wallet(app: &TestApp) -> Option<WalletId> {
    let before = CloudBackupStore::global().all_wallets().ok()?.into_iter().next()?;
    let mut after = before.clone();
    after.name.push('+');

    // save_all_wallets replaces every wallet of the network and mode, so the rest are saved
    // back unchanged; dropping them would delete wallets without the wallet-set change that
    // tells the inventory
    let wallets = Database::global().wallets();
    let all = wallets.get_all(after.network, after.wallet_mode).expect("read the wallets");
    let all = all
        .into_iter()
        .map(|wallet| if wallet.id == after.id { after.clone() } else { wallet })
        .collect();
    wallets
        .save_all_wallets(after.network, after.wallet_mode, all)
        .expect("save the changed wallet");
    app.manager().handle_wallet_metadata_update(&before, &after);
    Some(after.id)
}

/// The lifecycle record as the database holds it now
pub(crate) fn persisted() -> PersistedCloudBackupState {
    Database::global().cloud_backup_state.get().expect("read cloud backup state")
}

/// The namespace the keychain holds now
pub(crate) fn keychain_namespace() -> Option<String> {
    CloudBackupKeychain::global().namespace_id().expect("read namespace")
}

/// Load the detail inventory the way the detail screen does
pub(crate) fn load_detail(app: &mut TestApp) {
    app.dispatch(CloudBackupManagerAction::RefreshDetail).then_cannot(&[NextAction::Enable]);
    app.advance_clock(Duration::from_secs(60)).then_can(&[NextAction::TurnOff]);
}
