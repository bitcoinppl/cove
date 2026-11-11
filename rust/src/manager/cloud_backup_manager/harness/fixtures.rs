//! Small per-case accounts, plus the lived-in account for the incident replay

use std::time::Duration;

use cove_cspp::Cspp;
use cove_cspp::backup_data::{wallet_filename_from_record_id, wallet_record_id};
use cove_device::keychain::Keychain;
use cove_device::passkey::DiscoveredPasskeyResult;

use super::app::{NextAction, TestApp};
use crate::database::Database;
use crate::database::cloud_backup::PersistedCloudBackupState;
use crate::manager::cloud_backup_manager::ops::test_support::{
    enable_cloud_backup_without_reset, encrypted_wallet_backup_bytes_for_entry,
    wallet_entry_with_labels, xpub_only_wallet_metadata,
};
use crate::manager::cloud_backup_manager::owner::ui_test_legacy_record::legacy_stuck_record;
use crate::manager::cloud_backup_manager::{
    CloudBackupKeychain, CloudBackupManagerAction, CloudBackupStore,
};
use crate::wallet::metadata::WalletId;

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

        // real encrypted backups, so the detail inventory loads as it does for a user
        let target = Database::global().global_config.wallet_target();
        let wallet = || {
            let mut wallet = xpub_only_wallet_metadata();
            wallet.network = target.network;
            wallet
        };
        let local: Vec<_> = (0..self.local_wallets).map(|_| wallet()).collect();
        let elsewhere: Vec<_> = (0..self.not_on_device).map(|_| wallet()).collect();
        let not_on_device =
            elsewhere.iter().map(|wallet| wallet_record_id(wallet.id.as_ref())).collect();
        Database::global()
            .wallets()
            .save_all_wallets(target.network, target.mode, local.clone())
            .expect("save local wallets");

        let mut files = Vec::with_capacity(self.record_count());
        let mut local_records = Vec::with_capacity(local.len());
        for (index, wallet) in local.iter().cloned().chain(elsewhere).enumerate() {
            let record_id = wallet_record_id(wallet.id.as_ref());
            let entry = wallet_entry_with_labels(&wallet, None);
            if index < self.local_wallets {
                local_records.push(SeededRecord {
                    record_id: record_id.clone(),
                    revision: entry.content_revision_hash.clone(),
                });
            }

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

/// One seeded wallet backup and the content revision it was written at
#[derive(Debug, Clone)]
pub(crate) struct SeededRecord {
    pub(crate) record_id: String,
    pub(crate) revision: String,
}

/// The passkey every verifiable wrapper opens
pub(crate) const VERIFIABLE_PRF_KEY: [u8; 32] = [7; 32];
const VERIFIABLE_PRF_SALT: [u8; 32] = [9; 32];

/// The wrapper bytes the scripted passkey opens for `master_key`
pub(crate) fn verifiable_wrapper(master_key: &cove_cspp::master_key::MasterKey) -> Vec<u8> {
    let encrypted = cove_cspp::master_key_crypto::encrypt_master_key(
        master_key,
        &VERIFIABLE_PRF_KEY,
        &VERIFIABLE_PRF_SALT,
    )
    .expect("encrypt master key");

    serde_json::to_vec(&encrypted).expect("encode master key wrapper")
}

/// Replace the seeded placeholder wrapper with one the scripted passkey opens, and keep the
/// passkey's credential, so verification can recover a missing local master key
pub(crate) fn seed_verifiable_master_key(app: &TestApp, namespace: &str) {
    let master_key = Cspp::new(Keychain::global().clone())
        .load_master_key_from_store()
        .expect("read local master key")
        .expect("seeded master key");

    app.cloud.inner.set_master_key_backup(namespace.to_string(), verifiable_wrapper(&master_key));
    app.cloud.persist();
    CloudBackupKeychain::global()
        .save_passkey(&[1, 2, 3], VERIFIABLE_PRF_SALT)
        .expect("save passkey");
    app.passkey.inner.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: VERIFIABLE_PRF_KEY.to_vec(),
        credential_id: vec![1, 2, 3],
    }));
    app.passkey.inner.set_authenticate_result(Ok(VERIFIABLE_PRF_KEY.to_vec()));
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
