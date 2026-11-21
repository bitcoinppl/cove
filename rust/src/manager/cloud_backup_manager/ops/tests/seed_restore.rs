//! A batch cloud restore never skips or deletes a seed backup because a weaker local copy of the
//! same wallet exists
//!
//! Onboarding restore, Recover Other Backups and enable merge read many backups in one session.
//! A seed backup is skipped only when a local wallet holds that seed; a watch-only, cold,
//! hardware or keyless copy of the wallet gets the seed added or the backup restores beside it

use std::sync::Arc;

use bitcoin::secp256k1::PublicKey;
use cove_device::keychain::WalletSecret as KeychainWalletSecret;
use cove_tap_card::{TapSigner, TapSignerState};

use super::*;
use crate::manager::cloud_backup_manager::CloudBackupRestoreReport;
use crate::manager::cloud_backup_manager::wallets::WalletBackupReader;
use crate::wallet::fingerprint::Fingerprint;
use crate::wallet::metadata::{HardwareWalletMetadata, WalletId};
use crate::wallet_secret::WalletSecretExt as _;

/// One seed, and the wallets this device or the cloud can hold for it
struct Seed(Mnemonic);

impl Seed {
    fn new(entropy_byte: u8) -> Self {
        Self(Mnemonic::from_entropy(&[entropy_byte; 16]).unwrap())
    }

    fn secret(&self) -> KeychainWalletSecret {
        KeychainWalletSecret::Mnemonic(self.0.clone())
    }

    /// A wallet of this seed with its keychain items, the secret only for a hot one, and no row
    /// in the database
    fn wallet(&self, wallet_type: WalletType) -> WalletMetadata {
        self.wallet_with_id(wallet_type, WalletId::new())
    }

    fn wallet_with_id(&self, wallet_type: WalletType, id: WalletId) -> WalletMetadata {
        let mut metadata = WalletMetadata::preview_new();
        metadata.id = id;
        metadata.wallet_type = wallet_type;

        let secret = self.secret();
        let xpub = secret.xpub(metadata.network);
        let descriptors = secret.clone().into_descriptors(metadata.network, metadata.address_type);
        metadata.master_fingerprint = Some(Arc::new(Fingerprint::from(xpub.fingerprint())));

        let keychain = Keychain::global();
        keychain.save_wallet_xpub(&metadata.id, xpub).unwrap();
        keychain
            .save_public_descriptor(
                &metadata.id,
                descriptors.external.extended_descriptor,
                descriptors.internal.extended_descriptor,
            )
            .unwrap();

        if wallet_type == WalletType::Hot {
            keychain.save_wallet_secret(&metadata.id, secret).unwrap();
        }

        metadata
    }

    /// A wallet of this seed saved on this device
    fn local_wallet(&self, wallet_type: WalletType) -> WalletMetadata {
        let metadata = self.wallet(wallet_type);
        save_local_wallet(&metadata);

        metadata
    }

    /// The backup of a wallet of this seed, whose local data is gone again
    async fn cloud_backup(
        &self,
        wallet_type: WalletType,
        master_key: &cove_cspp::master_key::MasterKey,
    ) -> (WalletMetadata, Vec<u8>) {
        self.cloud_backup_with_id(wallet_type, WalletId::new(), master_key).await
    }

    async fn cloud_backup_with_id(
        &self,
        wallet_type: WalletType,
        id: WalletId,
        master_key: &cove_cspp::master_key::MasterKey,
    ) -> (WalletMetadata, Vec<u8>) {
        let metadata = self.wallet_with_id(wallet_type, id);
        let bytes =
            encrypted_remote_wallet_backup_bytes(&metadata, master_key, "seed-revision", 1).await;

        (metadata, bytes)
    }

    /// This seed's backup under `wallet_id`, the id another wallet of the seed already uses
    async fn cloud_backup_under_id(
        &self,
        wallet_id: &WalletId,
        master_key: &cove_cspp::master_key::MasterKey,
    ) -> Vec<u8> {
        let mut metadata = self.wallet(WalletType::Hot);
        let mut prepared = crate::manager::cloud_backup_manager::wallets::prepare_wallet_backup(
            &metadata,
            metadata.wallet_mode,
        )
        .await
        .unwrap();
        assert!(Keychain::global().delete_wallet_items(&metadata.id));

        metadata.id = wallet_id.clone();
        prepared.entry.wallet_id = wallet_id.to_string();
        prepared.entry.metadata = serde_json::to_value(&metadata).unwrap();

        encrypted_wallet_backup_bytes_for_entry(&prepared.entry, master_key, 1)
    }

    fn is_held_by(&self, wallet_id: &WalletId) -> bool {
        Keychain::global().get_wallet_secret(wallet_id).unwrap() == Some(self.secret())
    }
}

fn save_local_wallet(metadata: &WalletMetadata) {
    let wallets = Database::global().wallets();
    let mut saved = wallets.get_all(metadata.network, metadata.wallet_mode).unwrap();
    saved.push(metadata.clone());
    wallets.save_all_wallets(metadata.network, metadata.wallet_mode, saved).unwrap();
}

fn local_wallets() -> Vec<WalletMetadata> {
    let template = WalletMetadata::preview_new();
    Database::global().wallets().get_all(template.network, template.wallet_mode).unwrap()
}

fn local_wallet(id: &WalletId) -> WalletMetadata {
    local_wallets().into_iter().find(|wallet| &wallet.id == id).expect("the wallet is local")
}

/// The local wallets other than `known`, which a restore created
fn wallets_besides(known: &[&WalletId]) -> Vec<WalletMetadata> {
    local_wallets().into_iter().filter(|wallet| !known.contains(&&wallet.id)).collect()
}

/// A local hot wallet of `seed` whose stored secret no longer decrypts
fn local_wallet_with_lost_key(seed: &Seed) -> WalletMetadata {
    let metadata = seed.local_wallet(WalletType::Hot);

    let keychain = crate::test_support::shared_mock_keychain();
    let mut entries = keychain.entries_snapshot();
    let secret_key = format!("{}{}", metadata.id, crate::test_support::WALLET_MNEMONIC_KEY_SUFFIX);
    entries.insert(secret_key, "half-written-ciphertext".into());
    keychain.restore_entries(entries);
    assert!(Keychain::global().get_wallet_secret(&metadata.id).is_err());

    metadata
}

/// Turns `metadata`'s wallet into a TAPSIGNER wallet with the same public keys
fn make_tap_signer(metadata: &mut WalletMetadata) {
    // secp256k1 generator point G, a valid compressed public key
    let pubkey = PublicKey::from_slice(&[
        0x02, 0x79, 0xBE, 0x66, 0x7E, 0xF9, 0xDC, 0xBB, 0xAC, 0x55, 0xA0, 0x62, 0x95, 0xCE, 0x87,
        0x0B, 0x07, 0x02, 0x9B, 0xFC, 0xDB, 0x2D, 0xCE, 0x28, 0xD9, 0x59, 0xF2, 0x81, 0x5B, 0x16,
        0xF8, 0x17, 0x98,
    ])
    .unwrap();
    let tap_signer = Arc::new(TapSigner {
        state: TapSignerState::Sealed,
        card_ident: "TEST-IDENT".to_string(),
        nonce: "deadbeef".to_string(),
        signature: "cafebabe".to_string(),
        pubkey: Arc::new(pubkey),
    });

    metadata.wallet_type = WalletType::Cold;
    metadata.hardware_metadata = Some(HardwareWalletMetadata::TapSigner(tap_signer));
    Database::global().wallets.update_wallet_metadata(metadata.clone()).unwrap();
}

/// Two new wallet ids whose records a listing returns in this order, since it sorts them by
/// record id
fn listed_in_order() -> (WalletId, WalletId) {
    let (first, second) = (WalletId::new(), WalletId::new());
    if wallet_record_id(first.as_ref()) < wallet_record_id(second.as_ref()) {
        return (first, second);
    }

    (second, first)
}

/// Puts `backups` in `namespace`'s listing
fn list_backups(globals: &TestGlobals, namespace: &str, backups: Vec<(&WalletId, Vec<u8>)>) {
    let mut files = Vec::new();
    for (wallet_id, bytes) in backups {
        let record_id = wallet_record_id(wallet_id.as_ref());
        globals.cloud.set_wallet_backup(namespace.to_owned(), record_id.clone(), bytes);
        files.push(wallet_filename_from_record_id(&record_id));
    }

    globals.cloud.set_wallet_files(namespace.to_owned(), files);
}

/// A master key for a namespace that onboarding restore can open, locally and in the cloud
fn restorable_namespace(globals: &TestGlobals) -> cove_cspp::master_key::MasterKey {
    let master_key = cove_cspp::master_key::MasterKey::generate();
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &[7; 32], &[9; 32]).unwrap();
    globals.cloud.set_master_key_backup(
        master_key.namespace_id(),
        serde_json::to_vec(&encrypted_master).unwrap(),
    );
    cove_cspp::Cspp::new(Keychain::global().clone()).save_master_key(&master_key).unwrap();

    master_key
}

/// Another backup's master key, which Recover Other Backups opens with the discovered passkey
fn other_namespace(globals: &TestGlobals) -> cove_cspp::master_key::MasterKey {
    let prf_key = OTHER_PRF_KEY;
    let master_key = cove_cspp::master_key::MasterKey::generate();
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &prf_key, &[9; 32]).unwrap();
    globals.cloud.set_master_key_backup(
        master_key.namespace_id(),
        serde_json::to_vec(&encrypted_master).unwrap(),
    );
    offer_other_passkey(globals);

    master_key
}

const OTHER_PRF_KEY: [u8; 32] = [7; 32];

/// Lets the next Recover Other Backups discover the other backup's passkey
fn offer_other_passkey(globals: &TestGlobals) {
    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: OTHER_PRF_KEY.to_vec(),
        credential_id: vec![1, 2, 3],
    }));
}

fn active_namespace() -> String {
    CloudBackupKeychain::global().namespace_id().unwrap().unwrap()
}

fn active_master_key() -> cove_cspp::master_key::MasterKey {
    cove_cspp::Cspp::new(Keychain::global().clone()).load_master_key_from_store().unwrap().unwrap()
}

/// Whether the current backup's record of `wallet_id` holds a seed
async fn active_record_holds_seed(wallet_id: &WalletId) -> bool {
    let reader = WalletBackupReader::new(
        CloudStorage::global_explicit_client(),
        active_namespace(),
        zeroize::Zeroizing::new(active_master_key().critical_data_key()),
    );

    let wallet = reader.download(&wallet_record_id(wallet_id.as_ref())).await.unwrap();
    matches!(&wallet.entry.secret, WalletSecret::Mnemonic(_))
}

async fn onboarding_restore(manager: &RustCloudBackupManager) -> CloudBackupRestoreReport {
    let operation = new_restore_operation_for_test(manager).await;
    operation.restore_from_cloud_backup(manager).await.unwrap()
}

fn restored_and_failed(report: &CloudBackupRestoreReport) -> (u32, u32, Vec<String>) {
    (report.wallets_restored, report.wallets_failed, report.failed_wallet_errors.clone())
}

// the reported failure: a watch-only backup of the seed's wallet, listed first, made the seed
// backup look like a duplicate, so the seed stayed only in the cloud
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn onboarding_restore_brings_back_a_seed_whichever_order_its_watch_only_copy_lists() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    // a record whose metadata says hot but that holds no secret restores watch-only too
    for watch_only_type in [WalletType::Cold, WalletType::Hot] {
        reset_cloud_backup_test_state(&manager, globals);
        let master_key = restorable_namespace(globals);
        let seed = Seed::new(41);

        let (watch_only_id, hot_id) = listed_in_order();
        let (watch_only, watch_only_bytes) =
            seed.cloud_backup_with_id(WalletType::Cold, watch_only_id, &master_key).await;
        let watch_only_bytes = match watch_only_type {
            WalletType::Hot => {
                let reader = WalletBackupReader::new(
                    CloudStorage::global_explicit_client(),
                    String::new(),
                    zeroize::Zeroizing::new(master_key.critical_data_key()),
                );
                let encrypted = serde_json::from_slice(&watch_only_bytes).unwrap();
                let mut entry = reader.decrypt_entry(&encrypted).unwrap();
                let hot_typed =
                    WalletMetadata { wallet_type: WalletType::Hot, ..watch_only.clone() };
                entry.metadata = serde_json::to_value(&hot_typed).unwrap();

                encrypted_wallet_backup_bytes_for_entry(&entry, &master_key, 1)
            }
            _ => watch_only_bytes,
        };
        let (hot, hot_bytes) =
            seed.cloud_backup_with_id(WalletType::Hot, hot_id, &master_key).await;
        let relisted_hot_bytes = hot_bytes.clone();
        list_backups(
            globals,
            &master_key.namespace_id(),
            vec![(&watch_only.id, watch_only_bytes), (&hot.id, hot_bytes)],
        );

        let report = onboarding_restore(&manager).await;

        assert_eq!(restored_and_failed(&report), (2, 0, Vec::new()), "{watch_only_type:?}");
        let [restored] = local_wallets().try_into().unwrap_or_else(|wallets| {
            panic!("expected the watch-only wallet to take the seed, found {wallets:?}")
        });
        assert_eq!(restored.id, watch_only.id);
        assert_eq!(restored.wallet_type, WalletType::Hot);
        assert!(seed.is_held_by(&restored.id));

        // the wallet now holds the seed, so a second restore finds the seed on the device
        Database::global().cloud_backup_state.set(&PersistedCloudBackupState::Disabled).unwrap();
        list_backups(globals, &master_key.namespace_id(), vec![(&hot.id, relisted_hot_bytes)]);
        let second_report = onboarding_restore(&manager).await;
        assert_eq!(restored_and_failed(&second_report), (0, 0, Vec::new()));
        assert_eq!(local_wallets().len(), 1);
    }

    // listed the other way round, the seed restores first and the watch-only copy restores beside
    // it, which predates this fix: the seed is on the device either way
    reset_cloud_backup_test_state(&manager, globals);
    let master_key = restorable_namespace(globals);
    let seed = Seed::new(42);
    let (hot_id, watch_only_id) = listed_in_order();
    let (hot, hot_bytes) = seed.cloud_backup_with_id(WalletType::Hot, hot_id, &master_key).await;
    let (watch_only, watch_only_bytes) =
        seed.cloud_backup_with_id(WalletType::Cold, watch_only_id, &master_key).await;
    list_backups(
        globals,
        &master_key.namespace_id(),
        vec![(&hot.id, hot_bytes), (&watch_only.id, watch_only_bytes)],
    );

    let report = onboarding_restore(&manager).await;

    assert_eq!(restored_and_failed(&report), (2, 0, Vec::new()));
    assert_eq!(local_wallet(&hot.id).wallet_type, WalletType::Hot);
    assert!(seed.is_held_by(&hot.id));
    assert_eq!(local_wallet(&watch_only.id).wallet_type, WalletType::Cold);
    assert_eq!(local_wallets().len(), 2);
}

// a hot backup whose metadata lost its fingerprint is still the seed a local hot wallet holds
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn onboarding_restore_skips_a_seed_backup_without_a_fingerprint_whose_seed_is_local() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    reset_cloud_backup_test_state(&manager, globals);
    let master_key = restorable_namespace(globals);
    let seed = Seed::new(43);

    let local = seed.local_wallet(WalletType::Hot);
    let (mut backup, _) = seed.cloud_backup(WalletType::Hot, &master_key).await;
    backup.master_fingerprint = None;
    let backup_bytes = seed.cloud_backup_under_id(&backup.id, &master_key).await;
    let reader = WalletBackupReader::new(
        CloudStorage::global_explicit_client(),
        String::new(),
        zeroize::Zeroizing::new(master_key.critical_data_key()),
    );
    let mut entry = reader.decrypt_entry(&serde_json::from_slice(&backup_bytes).unwrap()).unwrap();
    entry.metadata = serde_json::to_value(&backup).unwrap();
    let backup_bytes = encrypted_wallet_backup_bytes_for_entry(&entry, &master_key, 1);
    list_backups(globals, &master_key.namespace_id(), vec![(&backup.id, backup_bytes)]);

    let report = onboarding_restore(&manager).await;

    assert_eq!(restored_and_failed(&report), (0, 0, Vec::new()));
    let [kept] = local_wallets()
        .try_into()
        .unwrap_or_else(|wallets| panic!("expected no second copy of the seed, found {wallets:?}"));
    assert_eq!(kept.id, local.id);
}

// adding a seed to a TAPSIGNER wallet would turn it hot and drop its card, so the seed restores
// as its own wallet and the TAPSIGNER wallet stays as it was
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn restore_never_adds_a_seed_to_a_hardware_wallet() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    reset_cloud_backup_test_state(&manager, globals);
    let master_key = restorable_namespace(globals);
    let seed = Seed::new(44);

    let mut tap_signer = seed.local_wallet(WalletType::Cold);
    make_tap_signer(&mut tap_signer);
    let (hot, hot_bytes) = seed.cloud_backup(WalletType::Hot, &master_key).await;
    list_backups(globals, &master_key.namespace_id(), vec![(&hot.id, hot_bytes)]);

    let report = onboarding_restore(&manager).await;

    assert_eq!(restored_and_failed(&report), (1, 0, Vec::new()));
    assert_eq!(local_wallet(&tap_signer.id), tap_signer);
    assert!(Keychain::global().get_wallet_secret(&tap_signer.id).unwrap().is_none());
    assert!(seed.is_held_by(&hot.id));
}

// two watch-only wallets of one seed leave no single wallet to add it to
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn restore_brings_a_seed_back_beside_two_watch_only_copies_of_it() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    reset_cloud_backup_test_state(&manager, globals);
    let master_key = restorable_namespace(globals);
    let seed = Seed::new(45);

    let first = seed.local_wallet(WalletType::Cold);
    let second = seed.local_wallet(WalletType::Cold);
    let (hot, hot_bytes) = seed.cloud_backup(WalletType::Hot, &master_key).await;
    list_backups(globals, &master_key.namespace_id(), vec![(&hot.id, hot_bytes)]);

    let report = onboarding_restore(&manager).await;

    assert_eq!(restored_and_failed(&report), (1, 0, Vec::new()));
    assert_eq!((local_wallet(&first.id), local_wallet(&second.id)), (first, second));
    assert!(seed.is_held_by(&hot.id));
    assert_eq!(local_wallets().len(), 3);
}

// the local wallet under the backup's id lost its key, so it can neither take the seed nor
// share its id, and the seed comes back under a new one
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn onboarding_restore_moves_a_seed_beside_a_local_wallet_that_lost_its_key() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    reset_cloud_backup_test_state(&manager, globals);
    let master_key = restorable_namespace(globals);
    let seed = Seed::new(46);

    let keyless = local_wallet_with_lost_key(&seed);
    let backup_bytes = seed.cloud_backup_under_id(&keyless.id, &master_key).await;
    list_backups(globals, &master_key.namespace_id(), vec![(&keyless.id, backup_bytes)]);
    let keychain_before = crate::test_support::raw_wallet_keychain_entries(&keyless.id);

    let report = onboarding_restore(&manager).await;

    assert_eq!(restored_and_failed(&report), (1, 0, Vec::new()));
    assert_eq!(local_wallet(&keyless.id), keyless);
    assert_eq!(crate::test_support::raw_wallet_keychain_entries(&keyless.id), keychain_before);
    let [restored] = wallets_besides(&[&keyless.id]).try_into().unwrap_or_else(|wallets| {
        panic!("expected the seed restored under a new id, found {wallets:?}")
    });
    assert!(seed.is_held_by(&restored.id));
}

#[tokio::test(flavor = "current_thread")]
async fn recover_other_backups_moves_a_seed_beside_a_local_wallet_that_lost_its_key() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    globals.cloud.set_reflect_uploaded_wallets_in_listing(true);
    let other_master_key = other_namespace(globals);
    let other = other_master_key.namespace_id();
    let seed = Seed::new(47);

    let keyless = local_wallet_with_lost_key(&seed);
    let backup_bytes = seed.cloud_backup_under_id(&keyless.id, &other_master_key).await;
    list_backups(globals, &other, vec![(&keyless.id, backup_bytes)]);
    let keychain_before = crate::test_support::raw_wallet_keychain_entries(&keyless.id);

    let report = recover_other_backups_for_test(&manager).await.unwrap();

    assert_eq!(restored_and_failed(&report), (1, 0, Vec::new()));
    assert_eq!(local_wallet(&keyless.id), keyless);
    assert_eq!(crate::test_support::raw_wallet_keychain_entries(&keyless.id), keychain_before);
    let [restored] = wallets_besides(&[&keyless.id]).try_into().unwrap_or_else(|wallets| {
        panic!("expected the seed restored under a new id, found {wallets:?}")
    });
    assert!(seed.is_held_by(&restored.id));

    // the restored wallet's own backup holds the seed, so the source is recovered
    assert!(active_record_holds_seed(&restored.id).await);
    assert!(!globals.cloud.has_namespace(&other));
}

#[tokio::test(flavor = "current_thread")]
async fn recover_other_backups_adds_a_foreign_seed_to_a_local_watch_only_wallet() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    globals.cloud.set_reflect_uploaded_wallets_in_listing(true);
    let other_master_key = other_namespace(globals);
    let other = other_master_key.namespace_id();
    let seed = Seed::new(48);

    let watch_only = seed.local_wallet(WalletType::Cold);
    let (hot, hot_bytes) = seed.cloud_backup(WalletType::Hot, &other_master_key).await;
    list_backups(globals, &other, vec![(&hot.id, hot_bytes)]);

    let report = recover_other_backups_for_test(&manager).await.unwrap();

    assert_eq!(restored_and_failed(&report), (1, 0, Vec::new()));
    let [upgraded] = local_wallets().try_into().unwrap_or_else(|wallets| {
        panic!("expected the watch-only wallet to take the seed, found {wallets:?}")
    });
    assert_eq!((upgraded.id.clone(), upgraded.wallet_type), (watch_only.id, WalletType::Hot));
    assert!(seed.is_held_by(&upgraded.id));
    assert!(active_record_holds_seed(&upgraded.id).await);

    // a label the wallet already had would keep its local text over the source's, so the
    // source stays
    assert!(globals.cloud.has_namespace(&other));
}

// the reported data loss: the current backup's watch-only record of a wallet shared its id with
// the other backup's seed record, so the seed record was skipped unread and its folder deleted
#[tokio::test(flavor = "current_thread")]
async fn recover_other_backups_reads_a_seed_record_sharing_an_id_with_a_watch_only_one() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    globals.cloud.set_reflect_uploaded_wallets_in_listing(true);
    let other_master_key = other_namespace(globals);
    let other = other_master_key.namespace_id();
    let seed = Seed::new(49);

    let watch_only = seed.local_wallet(WalletType::Cold);
    let watch_only_bytes =
        encrypted_wallet_backup_bytes(&watch_only, &active_master_key(), "watch-only", 1).await;
    list_backups(globals, &active_namespace(), vec![(&watch_only.id, watch_only_bytes)]);
    let seed_bytes = seed.cloud_backup_under_id(&watch_only.id, &other_master_key).await;
    list_backups(globals, &other, vec![(&watch_only.id, seed_bytes)]);

    let report = recover_other_backups_for_test(&manager).await.unwrap();

    assert_eq!(restored_and_failed(&report), (1, 0, Vec::new()));
    assert_eq!(local_wallet(&watch_only.id).wallet_type, WalletType::Hot);
    assert!(seed.is_held_by(&watch_only.id));
    assert!(active_record_holds_seed(&watch_only.id).await);
    assert!(globals.cloud.has_namespace(&other));

    // once the device holds the seed and the current backup a record under its id, the source
    // is proven recovered
    offer_other_passkey(globals);
    let second_report = recover_other_backups_for_test(&manager).await.unwrap();
    assert_eq!(restored_and_failed(&second_report), (0, 0, Vec::new()));
    assert!(!globals.cloud.has_namespace(&other));
}

// the upgraded wallet's backup stays watch-only until it uploads, so a failed batch upload still
// leaves it queued, even though a retry now finds the wallet covered and restores nothing
#[tokio::test(flavor = "current_thread")]
async fn recover_other_backups_queues_the_upgraded_wallet_when_its_upload_fails() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    let other_master_key = other_namespace(globals);
    let other = other_master_key.namespace_id();
    let seed = Seed::new(50);

    let watch_only = seed.local_wallet(WalletType::Cold);
    let (hot, hot_bytes) = seed.cloud_backup(WalletType::Hot, &other_master_key).await;
    list_backups(globals, &other, vec![(&hot.id, hot_bytes)]);
    globals.cloud.fail_wallet_backup_upload("upload failed");

    assert!(recover_other_backups_for_test(&manager).await.is_err());

    assert!(seed.is_held_by(&watch_only.id));
    let row = Database::global()
        .cloud_blob_sync_states
        .get(&wallet_record_id(watch_only.id.as_ref()))
        .unwrap();
    assert!(
        row.as_ref().is_some_and(PersistedCloudBlobSyncState::is_dirty),
        "the upgraded wallet should stay queued for upload, found {row:?}"
    );
    assert!(globals.cloud.has_namespace(&other));
}

#[tokio::test(flavor = "current_thread")]
async fn enable_merge_adds_a_backed_up_seed_to_a_local_watch_only_wallet() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    globals.cloud.set_reflect_uploaded_wallets_in_listing(true);

    let prf_key = [7u8; 32];
    let source_master_key = cove_cspp::master_key::MasterKey::generate();
    let active_master_key = cove_cspp::master_key::MasterKey::generate();
    let source = source_master_key.namespace_id();
    let active = active_master_key.namespace_id();
    for (namespace, master_key, salt) in
        [(&source, &source_master_key, [9; 32]), (&active, &active_master_key, [8; 32])]
    {
        let encrypted =
            cove_cspp::master_key_crypto::encrypt_master_key(master_key, &prf_key, &salt).unwrap();
        globals
            .cloud
            .set_master_key_backup(namespace.clone(), serde_json::to_vec(&encrypted).unwrap());
    }

    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: prf_key.to_vec(),
        credential_id: vec![1, 2, 3],
    }));
    globals.passkey.set_authenticate_result(Ok(prf_key.to_vec()));

    // the larger namespace becomes the active one, so it holds two unrelated wallets
    let first = Seed::new(51).cloud_backup(WalletType::Cold, &active_master_key).await;
    let second = Seed::new(52).cloud_backup(WalletType::Cold, &active_master_key).await;
    list_backups(globals, &active, vec![(&first.0.id, first.1), (&second.0.id, second.1)]);

    let seed = Seed::new(53);
    let watch_only = seed.local_wallet(WalletType::Cold);
    let (hot, hot_bytes) = seed.cloud_backup(WalletType::Hot, &source_master_key).await;
    list_backups(globals, &source, vec![(&hot.id, hot_bytes)]);

    enable_cloud_backup_create_new(&manager).await.unwrap();

    assert_eq!(active_namespace(), active);
    assert_eq!(local_wallet(&watch_only.id).wallet_type, WalletType::Hot);
    assert!(seed.is_held_by(&watch_only.id));
    assert!(active_record_holds_seed(&watch_only.id).await);
}

/// Rewrites `bytes`, a record encrypted with `master_key`, through `edit`
fn edit_record(
    bytes: &[u8],
    master_key: &cove_cspp::master_key::MasterKey,
    edit: impl FnOnce(&mut cove_cspp::backup_data::WalletEntry),
) -> Vec<u8> {
    let reader = WalletBackupReader::new(
        CloudStorage::global_explicit_client(),
        String::new(),
        zeroize::Zeroizing::new(master_key.critical_data_key()),
    );
    let mut entry = reader.decrypt_entry(&serde_json::from_slice(bytes).unwrap()).unwrap();
    edit(&mut entry);

    encrypted_wallet_backup_bytes_for_entry(&entry, master_key, 1)
}

/// This seed's hot backup under a new wallet id, with no master fingerprint in its metadata
async fn fingerprintless_seed_backup(
    seed: &Seed,
    master_key: &cove_cspp::master_key::MasterKey,
) -> (WalletId, Vec<u8>) {
    let mut metadata = seed.wallet(WalletType::Hot);
    assert!(Keychain::global().delete_wallet_items(&metadata.id));
    metadata.master_fingerprint = None;

    let bytes = seed.cloud_backup_under_id(&metadata.id, master_key).await;
    let bytes = edit_record(&bytes, master_key, |entry| {
        entry.metadata = serde_json::to_value(&metadata).unwrap();
    });

    (metadata.id, bytes)
}

// the current backup's labels can still differ from the source's once the seed is added, since a
// label the wallet already had keeps its local text, so a later Recover, which finds the seed
// local and a record under the same id in the current backup, keeps the source until the current
// backup holds the same labels
#[tokio::test(flavor = "current_thread")]
async fn recover_other_backups_keeps_a_seed_source_whose_labels_the_current_backup_lacks() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    globals.cloud.set_reflect_uploaded_wallets_in_listing(true);
    let other_master_key = other_namespace(globals);
    let other = other_master_key.namespace_id();
    let seed = Seed::new(51);

    let watch_only = seed.local_wallet(WalletType::Cold);
    let watch_only_bytes =
        encrypted_wallet_backup_bytes(&watch_only, &active_master_key(), "watch-only", 1).await;
    list_backups(globals, &active_namespace(), vec![(&watch_only.id, watch_only_bytes)]);
    let seed_bytes = seed.cloud_backup_under_id(&watch_only.id, &other_master_key).await;
    let seed_bytes = edit_record(&seed_bytes, &other_master_key, |entry| {
        entry.labels_count = 1;
        entry.labels_hash = Some("labels-only-the-source-holds".into());
    });
    list_backups(globals, &other, vec![(&watch_only.id, seed_bytes)]);

    recover_other_backups_for_test(&manager).await.unwrap();
    assert!(seed.is_held_by(&watch_only.id));
    assert!(active_record_holds_seed(&watch_only.id).await);

    offer_other_passkey(globals);
    let _ = recover_other_backups_for_test(&manager).await;

    assert!(globals.cloud.has_namespace(&other));
}

// the current backup's copy is the newer one this device uploads over, so an older source copy
// of a wallet that is not on this device is never restored and uploaded over it
#[tokio::test(flavor = "current_thread")]
async fn recover_other_backups_never_uploads_an_older_copy_over_the_current_one() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    let other_master_key = other_namespace(globals);
    let other = other_master_key.namespace_id();
    let seed = Seed::new(52);

    let (current, current_bytes) = seed.cloud_backup(WalletType::Hot, &active_master_key()).await;
    list_backups(globals, &active_namespace(), vec![(&current.id, current_bytes.clone())]);
    let older_bytes = seed.cloud_backup_under_id(&current.id, &other_master_key).await;
    list_backups(globals, &other, vec![(&current.id, older_bytes)]);

    let _ = recover_other_backups_for_test(&manager).await;

    let record_id = wallet_record_id(current.id.as_ref());
    assert_eq!(
        globals.cloud.wallet_backup_bytes(&active_namespace(), &record_id),
        Some(current_bytes)
    );
    assert!(local_wallets().is_empty(), "nothing should be restored from the older copy");
}

// a restored seed wallet keeps the fingerprint of its seed even when its record had none, so a
// later run finds the seed local instead of restoring it again under a new wallet id
#[tokio::test(flavor = "current_thread")]
async fn recover_other_backups_restores_a_fingerprintless_seed_backup_only_once() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    globals.cloud.set_reflect_uploaded_wallets_in_listing(true);
    let seed = Seed::new(53);

    let first_master_key = other_namespace(globals);
    let (first_id, first_bytes) = fingerprintless_seed_backup(&seed, &first_master_key).await;
    list_backups(globals, &first_master_key.namespace_id(), vec![(&first_id, first_bytes)]);
    recover_other_backups_for_test(&manager).await.unwrap();
    assert_eq!(local_wallets().len(), 1);

    let second_master_key = other_namespace(globals);
    let (second_id, second_bytes) = fingerprintless_seed_backup(&seed, &second_master_key).await;
    list_backups(globals, &second_master_key.namespace_id(), vec![(&second_id, second_bytes)]);
    let _ = recover_other_backups_for_test(&manager).await;

    let [restored] = local_wallets()
        .try_into()
        .unwrap_or_else(|wallets| panic!("expected one wallet for the seed, found {wallets:?}"));
    assert!(seed.is_held_by(&restored.id));
}

// a wallet restored under a new wallet id has no backup until it uploads, and a retry finds its
// seed local, so a failed batch upload still leaves it queued
#[tokio::test(flavor = "current_thread")]
async fn recover_other_backups_queues_a_seed_moved_to_a_new_id_when_its_upload_fails() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    let other_master_key = other_namespace(globals);
    let other = other_master_key.namespace_id();
    let seed = Seed::new(54);

    let keyless = local_wallet_with_lost_key(&seed);
    let backup_bytes = seed.cloud_backup_under_id(&keyless.id, &other_master_key).await;
    list_backups(globals, &other, vec![(&keyless.id, backup_bytes)]);
    globals.cloud.fail_wallet_backup_upload("upload failed");

    assert!(recover_other_backups_for_test(&manager).await.is_err());

    let [restored] = wallets_besides(&[&keyless.id]).try_into().unwrap_or_else(|wallets| {
        panic!("expected the seed restored under a new id, found {wallets:?}")
    });
    assert!(seed.is_held_by(&restored.id));
    let row = Database::global()
        .cloud_blob_sync_states
        .get(&wallet_record_id(restored.id.as_ref()))
        .unwrap();
    assert!(
        row.as_ref().is_some_and(PersistedCloudBlobSyncState::is_dirty),
        "the restored wallet should stay queued for upload, found {row:?}"
    );
    assert!(globals.cloud.has_namespace(&other));
}

// a watch-only wallet keeps its wallet id and xpub when its address type changes, so a current
// record under the same id can watch other descriptors than the source, which then stays
#[tokio::test(flavor = "current_thread")]
async fn recover_other_backups_keeps_a_watch_only_source_whose_descriptors_differ() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    let other_master_key = other_namespace(globals);
    let other = other_master_key.namespace_id();
    let seed = Seed::new(55);

    let local = seed.local_wallet(WalletType::Cold);
    let current_bytes =
        encrypted_wallet_backup_bytes(&local, &active_master_key(), "current", 1).await;
    list_backups(globals, &active_namespace(), vec![(&local.id, current_bytes)]);

    // the same wallet and xpub, backed up before its address type changed
    let mut before_change = local.clone();
    before_change.address_type = crate::wallet::WalletAddressType::WrappedSegwit;
    let descriptors =
        seed.secret().into_descriptors(before_change.network, before_change.address_type);
    let source_bytes = encrypted_wallet_backup_bytes(&local, &other_master_key, "source", 1).await;
    let source_bytes = edit_record(&source_bytes, &other_master_key, |entry| {
        entry.metadata = serde_json::to_value(&before_change).unwrap();
        entry.descriptors = Some(cove_cspp::backup_data::DescriptorPair {
            external: descriptors.external.extended_descriptor.to_string(),
            internal: descriptors.internal.extended_descriptor.to_string(),
        });
    });
    list_backups(globals, &other, vec![(&local.id, source_bytes)]);

    let error = recover_other_backups_for_test(&manager).await.unwrap_err();

    // the record was read and compared, found nothing to restore, and kept its namespace
    assert!(error.to_string().contains("no wallets were found"), "{error}");
    assert!(globals.cloud.has_namespace(&other));
    assert_eq!(local_wallets(), vec![local]);
}

// a wallet restored under its own id from another namespace has no record in the current
// backup until it uploads, and a retry finds it local, so a failed batch upload leaves it queued
#[tokio::test(flavor = "current_thread")]
async fn recover_other_backups_queues_a_restored_seed_when_its_upload_fails() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    let other_master_key = other_namespace(globals);
    let other = other_master_key.namespace_id();
    let seed = Seed::new(57);

    let (hot, hot_bytes) = seed.cloud_backup(WalletType::Hot, &other_master_key).await;
    list_backups(globals, &other, vec![(&hot.id, hot_bytes)]);
    globals.cloud.fail_wallet_backup_upload("upload failed");

    assert!(recover_other_backups_for_test(&manager).await.is_err());

    assert!(seed.is_held_by(&hot.id));
    let row =
        Database::global().cloud_blob_sync_states.get(&wallet_record_id(hot.id.as_ref())).unwrap();
    assert!(
        row.as_ref().is_some_and(PersistedCloudBlobSyncState::is_dirty),
        "the restored wallet should stay queued for upload, found {row:?}"
    );
    assert!(globals.cloud.has_namespace(&other));
}

const CURRENT_ONLY_TXID: &str = "d97bf8892657980426c879e4ab2001f09342f1ab61cfa602741a7715a3d60290";
const SEED_ONLY_TXID: &str = "f91d0a8a78462bc59398f2c5d7a84fcff491c26ba54c4833478b202796c8aafd";
const SHARED_TXID: &str = "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b";

fn tx_label(txid: &str, label: &str) -> String {
    format!(r#"{{"type":"tx","ref":"{txid}","label":"{label}"}}"#)
}

/// Gives `entry` exactly the labels in `jsonl`
fn set_record_labels(entry: &mut cove_cspp::backup_data::WalletEntry, jsonl: &str) {
    let labeled = wallet_entry_with_labels(&WalletMetadata::preview_new(), Some(jsonl));
    entry.labels_zstd_jsonl = labeled.labels_zstd_jsonl.clone();
    entry.labels_count = labeled.labels_count;
    entry.labels_hash = labeled.labels_hash.clone();
    entry.labels_uncompressed_size = labeled.labels_uncompressed_size;
}

/// The labels the current backup's record of `wallet_id` holds
async fn active_record_labels(wallet_id: &WalletId) -> String {
    let reader = WalletBackupReader::new(
        CloudStorage::global_explicit_client(),
        active_namespace(),
        zeroize::Zeroizing::new(active_master_key().critical_data_key()),
    );

    let wallet = reader.download(&wallet_record_id(wallet_id.as_ref())).await.unwrap();
    crate::manager::cloud_backup_manager::wallets::decode_cloud_labels_jsonl(&wallet.entry)
        .unwrap()
        .unwrap_or_default()
}

// adding the seed uploads the local wallet over the current backup's record under its id, so
// labels that only that record or the seed backup held are added to the wallet first, while a
// label edited on this device keeps its local text
#[tokio::test(flavor = "current_thread")]
async fn recover_other_backups_keeps_cloud_labels_when_adding_a_seed_to_a_local_wallet() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    globals.cloud.set_reflect_uploaded_wallets_in_listing(true);
    let other_master_key = other_namespace(globals);
    let other = other_master_key.namespace_id();
    let seed = Seed::new(56);

    let watch_only = seed.local_wallet(WalletType::Cold);
    LabelManager::try_new(watch_only.id.clone())
        .unwrap()
        .import_without_cloud_backup_dirty(&tx_label(SHARED_TXID, "edited on this device"))
        .unwrap();

    let current_labels = [
        tx_label(CURRENT_ONLY_TXID, "only in the current backup"),
        tx_label(SHARED_TXID, "older current text"),
    ]
    .join("\n");
    let master_key = active_master_key();
    let watch_only_bytes =
        encrypted_wallet_backup_bytes(&watch_only, &master_key, "watch-only", 1).await;
    let watch_only_bytes = edit_record(&watch_only_bytes, &master_key, |entry| {
        set_record_labels(entry, &current_labels);
    });
    list_backups(globals, &active_namespace(), vec![(&watch_only.id, watch_only_bytes)]);

    let seed_labels = [
        tx_label(SEED_ONLY_TXID, "only in the seed backup"),
        tx_label(SHARED_TXID, "older seed text"),
    ]
    .join("\n");
    let seed_bytes = seed.cloud_backup_under_id(&watch_only.id, &other_master_key).await;
    let seed_bytes = edit_record(&seed_bytes, &other_master_key, |entry| {
        set_record_labels(entry, &seed_labels);
    });
    list_backups(globals, &other, vec![(&watch_only.id, seed_bytes)]);

    let report = recover_other_backups_for_test(&manager).await.unwrap();

    assert_eq!(restored_and_failed(&report), (1, 0, Vec::new()));
    assert!(report.labels_failed_wallet_names.is_empty());
    assert!(seed.is_held_by(&watch_only.id));

    let local_labels =
        LabelManager::try_new(watch_only.id.clone()).unwrap().export().await.unwrap();
    let uploaded_labels = active_record_labels(&watch_only.id).await;
    for labels in [&local_labels, &uploaded_labels] {
        assert!(labels.contains("only in the current backup"), "missing current label: {labels}");
        assert!(labels.contains("only in the seed backup"), "missing seed label: {labels}");
        assert!(labels.contains("edited on this device"), "local edit replaced: {labels}");
        assert!(!labels.contains("older"), "an older copy replaced a local edit: {labels}");
    }
}
