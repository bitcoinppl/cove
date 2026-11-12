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
use crate::database::cloud_backup::{PersistedCloudBackupState, PersistedCloudBlobState};
use crate::manager::cloud_backup_manager::ops::test_support::{
    enable_cloud_backup_without_reset, encrypted_wallet_backup_bytes_for_entry,
    wallet_entry_with_labels, xpub_only_wallet_metadata,
};
use crate::manager::cloud_backup_manager::owner::ui_test_legacy_record::legacy_stuck_record;
use crate::manager::cloud_backup_manager::{
    CloudBackupKeychain, CloudBackupManagerAction, CloudBackupResetPermit, CloudBackupStore,
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
        let mut local_records = Vec::with_capacity(local.len());
        for (index, wallet) in local.into_iter().chain(elsewhere).enumerate() {
            let record_id = wallet_record_id(wallet.metadata.id.as_ref());
            let mut entry = wallet_entry_with_labels(&wallet.metadata, None);
            entry.xpub = Some(wallet.xpub.to_string());
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

/// Seeds a configured backup of one wallet and uploads a change to that wallet, so the wallet has
/// an upload row in the seeded namespace, which this returns
pub(crate) fn seed_an_uploaded_wallet(app: &mut TestApp) -> String {
    let namespace = FakeAccount::new(1, 0).seed(app);
    load_detail(app);

    app.event("a local wallet changed", |app| {
        change_a_wallet(app).expect("a seeded wallet changed");
    })
    .then_can(&[NextAction::TurnOff]);
    app.advance_clock(Duration::from_secs(60)).then_can(&[NextAction::TurnOff]);

    let rows = Database::global().cloud_blob_sync_states.list().expect("read the upload rows");
    assert!(
        rows.iter().any(|row| row.wallet_id().is_some() && row.namespace_id == namespace),
        "the changed wallet has an upload row in {namespace}: {rows:#?}"
    );

    namespace
}

/// Every upload row names `namespace`, and every wallet's upload there is confirmed
pub(crate) fn assert_uploads_confirmed_in(app: &TestApp, namespace: &str) {
    let rows = Database::global().cloud_blob_sync_states.list().expect("read the upload rows");
    assert!(rows.iter().any(|row| row.wallet_id().is_some()), "no wallet upload row: {rows:#?}");

    for row in rows {
        assert_eq!(row.namespace_id, namespace, "{row:#?}\n{}", app.describe());
        if row.wallet_id().is_some() {
            assert!(
                matches!(row.state, PersistedCloudBlobState::Confirmed(_)),
                "{row:#?}\n{}",
                app.describe()
            );
        }
    }
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

/// Runs the Cloud Backup part of a full wipe: the owner clears local state, the database reset
/// commits, and the reset finishes
pub(crate) fn finish_wipe(app: &TestApp, permit: CloudBackupResetPermit) {
    app.block_on("the owner to clear local state", permit.clear_local_state())
        .expect("clear local state");
    // the wipe's commit point: `wipe_all_data_with_tier` resets the whole database, so every
    // Cloud Backup table, the blob sync rows included, is gone before the reset finishes
    Database::global().dangerous_reset_all_data().expect("reset the database");
    app.block_on("the reset to finish", permit.complete_after_database_reset())
        .expect("finish the reset");
}
