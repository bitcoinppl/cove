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
    CloudBackupWalletVerificationIssues, DeepVerificationReport, VerificationState,
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

/// Writes `count` backups of wallets this device doesn't have under a key it doesn't hold, lists
/// them beside `account`'s records, and reports them the way a verification that couldn't
/// decrypt them does
///
/// Returns their record ids in record order, the order a batch delete takes them in
pub(crate) fn seed_undecryptable_records(
    app: &TestApp,
    account: &SeededAccount,
    count: usize,
) -> Vec<String> {
    let other_key = cove_cspp::master_key::MasterKey::generate();
    let target = Database::global().global_config.wallet_target();
    let mut record_ids = Vec::with_capacity(count);
    for _ in 0..count {
        let wallet = DistinctWallet::new(target.network);
        let record_id = wallet_record_id(wallet.metadata.id.as_ref());
        let mut entry = wallet_entry_with_labels(&wallet.metadata, None);
        entry.xpub = Some(wallet.xpub.to_string());

        let backup = encrypted_wallet_backup_bytes_for_entry(&entry, &other_key, 1);
        app.cloud.inner.set_wallet_backup(account.namespace.clone(), record_id.clone(), backup);
        record_ids.push(record_id);
    }

    record_ids.sort();
    let files = account
        .local
        .iter()
        .map(|record| record.record_id.as_str())
        .chain(account.not_on_device.iter().map(String::as_str))
        .chain(record_ids.iter().map(String::as_str))
        .map(wallet_filename_from_record_id)
        .collect();

    app.cloud.inner.set_wallet_files(account.namespace.clone(), files);
    app.cloud.persist();

    let decryption_failed = u32::try_from(count).expect("record count fits u32");
    app.manager().apply_verification_state(VerificationState::NeedsAttention(
        DeepVerificationReport {
            master_key_wrapper_repaired: false,
            local_master_key_repaired: false,
            credential_recovered: false,
            wallets_verified: 0,
            wallet_issues: CloudBackupWalletVerificationIssues {
                decryption_failed,
                ..CloudBackupWalletVerificationIssues::default()
            },
            detail: None,
        },
    ));
    app.settle();

    record_ids
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

/// Put one wallet's backup in `namespace`, encrypted with `master_key`, for a wallet not on this
/// device, so a Restore of that backup has a wallet to bring back
pub(crate) fn seed_wallet_backup_elsewhere(
    app: &TestApp,
    namespace: &str,
    master_key: &cove_cspp::master_key::MasterKey,
) {
    let target = Database::global().global_config.wallet_target();
    let wallet = DistinctWallet::new(target.network);
    let record_id = wallet_record_id(wallet.metadata.id.as_ref());
    let mut entry = wallet_entry_with_labels(&wallet.metadata, None);
    entry.xpub = Some(wallet.xpub.to_string());

    let backup = encrypted_wallet_backup_bytes_for_entry(&entry, master_key, 1);
    app.cloud.inner.set_wallet_backup(namespace.to_owned(), record_id.clone(), backup);
    app.cloud
        .inner
        .set_wallet_files(namespace.to_owned(), vec![wallet_filename_from_record_id(&record_id)]);
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
        .save_passkey_and_hint(&[1, 2, 3], VERIFIABLE_PRF_SALT, None)
        .expect("save passkey");
    app.passkey.inner.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: VERIFIABLE_PRF_KEY.to_vec(),
        credential_id: vec![1, 2, 3],
    }));
    app.passkey.inner.set_authenticate_result(Ok(VERIFIABLE_PRF_KEY.to_vec()));
}

/// Saves `credential_id` as this device's passkey with the salt the verifiable wrapper uses, and
/// where it was created when `hint` is given
pub(crate) fn save_device_passkey(
    credential_id: &[u8],
    hint: Option<&cove_cspp::backup_data::PasskeyProviderHint>,
) {
    CloudBackupKeychain::global()
        .save_passkey_and_hint(credential_id, VERIFIABLE_PRF_SALT, hint)
        .expect("save passkey");
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

/// How a record [`seed_unreadable_records`] lists fails to read with this device's key
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnreadableSeed {
    /// A real wallet backup encrypted under another master key: a decrypt failure
    OtherKey,
    /// Bytes that are not a wallet backup at all, so reading fails before any key is used
    Garbage,
    /// Listed by the provider with nothing behind it to download
    Missing,
}

/// Lists one record of each [`UnreadableSeed`] in `namespace`, beside what it already lists, and
/// returns their record ids with how each fails
pub(crate) fn seed_unreadable_records(
    app: &TestApp,
    namespace: &str,
) -> Vec<(String, UnreadableSeed)> {
    let other_key = cove_cspp::master_key::MasterKey::generate();
    let network = Database::global().global_config.wallet_target().network;
    let mut files =
        app.cloud.inner.durable_contents().wallet_files.remove(namespace).unwrap_or_default();

    let mut seeded = Vec::new();
    for seed in [UnreadableSeed::OtherKey, UnreadableSeed::Garbage, UnreadableSeed::Missing] {
        let mut wallet = xpub_only_wallet_metadata();
        wallet.network = network;
        let record_id = wallet_record_id(wallet.id.as_ref());

        let bytes = match seed {
            UnreadableSeed::OtherKey => {
                let entry = wallet_entry_with_labels(&wallet, None);
                Some(encrypted_wallet_backup_bytes_for_entry(&entry, &other_key, 1))
            }
            UnreadableSeed::Garbage => Some(b"not a wallet backup".to_vec()),
            UnreadableSeed::Missing => None,
        };

        if let Some(bytes) = bytes {
            app.cloud.inner.set_wallet_backup(namespace.to_owned(), record_id.clone(), bytes);
        }

        files.push(wallet_filename_from_record_id(&record_id));
        seeded.push((record_id, seed));
    }

    app.cloud.inner.set_wallet_files(namespace.to_owned(), files);
    app.cloud.persist();
    seeded
}

/// Seed a second namespace in the cloud, as an earlier Enable on another device would leave
/// it, holding one wallet that is not on this device
///
/// Its master key is wrapped with the scripted passkey's PRF, so only a discovered passkey
/// opens it. Returns the namespace and the wallet it holds
pub(crate) fn seed_other_namespace(app: &TestApp) -> (String, WalletId) {
    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace = master_key.namespace_id();

    let mut wallet = xpub_only_wallet_metadata();
    wallet.network = Database::global().global_config.wallet_target().network;
    let record_id = wallet_record_id(wallet.id.as_ref());
    let entry = wallet_entry_with_labels(&wallet, None);
    let backup = encrypted_wallet_backup_bytes_for_entry(&entry, &master_key, 1);

    app.cloud.inner.set_master_key_backup(namespace.clone(), verifiable_wrapper(&master_key));
    app.cloud.inner.set_wallet_backup(namespace.clone(), record_id.clone(), backup);
    app.cloud
        .inner
        .set_wallet_files(namespace.clone(), vec![wallet_filename_from_record_id(&record_id)]);
    app.cloud.persist();

    (namespace, wallet.id)
}
