use std::{
    collections::BTreeMap,
    fs,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    str::FromStr as _,
};

use super::{
    BackupError, Database, Keychain, KeychainArtifactKind, MAX_WALLET_ID_BYTES,
    PersistedArtifactSnapshot, PersistedRestoreMarker, QUARANTINE_DIRECTORY_NAME,
    RESTORE_MARKER_VERSION, RestoreArtifactSnapshot, RestoreFileLock, RestoreMarkerGuard,
    RestoreMarkerPhase, ValidatedRestoreWalletId, WalletId, WalletMetadata, WalletRestoreLease,
    hash_wallet_data_entry, lock_path, marker_directory, marker_path, operation_id,
    recover_restore_markers, remove_marker, rollback_keychain, secret_fingerprint,
    value_fingerprint, write_marker,
};

fn id(value: &str) -> WalletId {
    WalletId::from(value.to_string())
}

fn write_test_marker(
    wallet_id: &WalletId,
    snapshot: &RestoreArtifactSnapshot,
    phase: RestoreMarkerPhase,
) -> PathBuf {
    let operation_id = operation_id();
    let path = marker_path(&operation_id).unwrap();
    let marker = PersistedRestoreMarker {
        version: RESTORE_MARKER_VERSION,
        operation_id,
        wallet_id: wallet_id.as_str().to_string(),
        initial: PersistedArtifactSnapshot::from_snapshot(snapshot),
        phase,
    };
    write_marker(&path, &marker).unwrap();
    path
}

#[test]
fn restore_ids_reject_path_and_control_input() {
    for value in ["", ".", "..", "a/b", "a\\b", "a\0b", "a\n"] {
        assert!(ValidatedRestoreWalletId::validate(&id(value)).is_err(), "{value:?}");
    }
}

#[test]
fn restore_ids_reject_unsafe_length_and_suffixes() {
    assert!(ValidatedRestoreWalletId::validate(&id(&"a".repeat(MAX_WALLET_ID_BYTES + 1))).is_err());
    assert!(ValidatedRestoreWalletId::validate(&id("wallet.")).is_err());
    assert!(ValidatedRestoreWalletId::validate(&id("wallet ")).is_err());
    assert!(ValidatedRestoreWalletId::validate(&id("wallet:name")).is_err());
    assert!(ValidatedRestoreWalletId::validate(&id("wallet")).is_ok());
}

#[test]
fn restore_ids_reject_non_nanoid_unicode() {
    for value in ["walleté", "wallete\u{301}", "钱包"] {
        assert!(ValidatedRestoreWalletId::validate(&id(value)).is_err(), "{value:?}");
    }

    assert!(ValidatedRestoreWalletId::validate(&id("Wallet_01-test")).is_ok());
}

#[test]
fn persisted_marker_stores_no_secret_or_filename() {
    let mnemonic = bip39::Mnemonic::from_str(
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
    )
    .unwrap();
    let snapshot = RestoreArtifactSnapshot {
        keychain_items: true,
        keychain_entries: BTreeMap::from([(
            KeychainArtifactKind::Secret.as_str().to_string(),
            Some(secret_fingerprint("mnemonic:", &mnemonic)),
        )]),
        wallet_data_fingerprints: BTreeMap::from([(
            hash_wallet_data_entry(Path::new("secret-labels.redb")),
            value_fingerprint(b"labels"),
        )]),
        ..RestoreArtifactSnapshot::default()
    };

    let marker = PersistedArtifactSnapshot::from_snapshot(&snapshot);
    let json = serde_json::to_string(&marker).unwrap();

    assert!(!json.contains("secret-labels.redb"));
    assert!(json.contains(KeychainArtifactKind::Secret.as_str()));
    assert!(
        !json.contains(&secret_fingerprint("mnemonic:", &mnemonic).digest),
        "the marker must not persist a digest that verifies a seed: {json}"
    );
}

#[test]
fn committed_restore_reports_marker_cleanup_warning() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    crate::database::test_support::delete_database();
    crate::test_support::init_test_keychain();
    crate::test_support::shared_mock_keychain().reset();

    let metadata = WalletMetadata::preview_new();
    let journal =
        RestoreMarkerGuard::test_begin_with_snapshot(&metadata, RestoreArtifactSnapshot::default());
    let marker_path = journal.marker_path.clone();
    // a directory cannot be unlinked, so marker cleanup fails while the
    // restore itself stays committed
    fs::remove_file(&marker_path).unwrap();
    fs::create_dir(&marker_path).unwrap();

    let warnings = journal.commit();

    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("cleanup is pending"));
    fs::remove_dir(&marker_path).unwrap();
}

#[test]
fn marker_drop_rolls_back_new_artifacts_during_panic_unwind() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    crate::test_support::init_test_keychain();
    crate::test_support::shared_mock_keychain().reset();

    let mut metadata = WalletMetadata::preview_new();
    metadata.name = "panic restore".to_string();
    let artifact = crate::bdk_store::BdkStore::wallet_store_artifact_paths(&metadata.id)
        .into_iter()
        .find(|path| path.to_string_lossy().ends_with("-wal"))
        .unwrap();
    if let Some(parent) = artifact.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let _ = fs::remove_file(&artifact);

    let result = catch_unwind(AssertUnwindSafe(|| {
        let _journal = RestoreMarkerGuard::test_begin_with_snapshot(
            &metadata,
            RestoreArtifactSnapshot { metadata: true, ..RestoreArtifactSnapshot::default() },
        );
        fs::write(&artifact, b"created during restore").unwrap();
        panic!("simulate restore panic");
    }));

    assert!(result.is_err());
    assert!(!artifact.exists());
}

#[test]
fn approved_cleanup_marks_keychain_artifacts_as_owned() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    crate::database::test_support::delete_database();
    crate::app::reconcile::test_support::init_noop_updater();
    crate::test_support::init_test_keychain();
    crate::test_support::shared_mock_keychain().reset();

    let metadata = WalletMetadata::preview_new();
    let xpub = bitcoin::bip32::Xpub::from_str(
        "xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM",
    )
    .unwrap();
    Keychain::global().save_wallet_xpub(&metadata.id, xpub).unwrap();

    let validated = ValidatedRestoreWalletId::validate(&metadata.id).unwrap();
    let snapshot = RestoreArtifactSnapshot::capture(&validated).unwrap();

    let mut journal = RestoreMarkerGuard::test_begin_with_snapshot(&metadata, snapshot);
    journal.remove_approved_conflicts().unwrap();
    assert!(!Keychain::global().wallet_items_exist(&metadata.id));

    Keychain::global().save_wallet_xpub(&metadata.id, xpub).unwrap();
    assert!(journal.rollback().is_empty());
    assert!(!Keychain::global().wallet_items_exist(&metadata.id));
}

#[test]
fn rollback_preserves_existing_xpub_and_removes_added_secret() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    crate::database::test_support::delete_database();
    crate::app::reconcile::test_support::init_noop_updater();
    crate::test_support::init_test_keychain();
    crate::test_support::shared_mock_keychain().reset();

    let metadata = WalletMetadata::preview_new();
    let xpub = bitcoin::bip32::Xpub::from_str(
        "xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM",
    )
    .unwrap();
    Keychain::global().save_wallet_xpub(&metadata.id, xpub).unwrap();

    let validated = ValidatedRestoreWalletId::validate(&metadata.id).unwrap();
    let snapshot = RestoreArtifactSnapshot::capture(&validated).unwrap();
    let mut journal = RestoreMarkerGuard::test_begin_with_snapshot(&metadata, snapshot);
    let mnemonic = bip39::Mnemonic::from_str(
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
    )
    .unwrap();
    Keychain::global().save_wallet_key(&metadata.id, mnemonic).unwrap();

    assert!(journal.rollback().is_empty());
    assert_eq!(Keychain::global().get_wallet_xpub(&metadata.id).unwrap(), Some(xpub));
    assert!(Keychain::global().get_wallet_secret(&metadata.id).unwrap().is_none());

    assert!(Keychain::global().delete_wallet_xpub(&metadata.id));
}

#[test]
fn approval_lease_rejects_changed_artifact_snapshot() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    crate::database::test_support::delete_database();
    crate::app::reconcile::test_support::init_noop_updater();
    crate::test_support::init_test_keychain();
    crate::test_support::shared_mock_keychain().reset();

    let metadata = WalletMetadata::preview_new();
    let validated = ValidatedRestoreWalletId::validate(&metadata.id).unwrap();
    let expected = RestoreArtifactSnapshot::capture(&validated).unwrap();
    let artifact = crate::bdk_store::BdkStore::wallet_store_artifact_paths(&metadata.id)[1].clone();
    fs::write(&artifact, b"changed after approval").unwrap();

    assert!(matches!(
        WalletRestoreLease::acquire_for_approval(&metadata, &expected),
        Err(BackupError::ImportApprovalStale(id)) if id == metadata.id
    ));
    fs::remove_file(artifact).unwrap();
}

#[test]
fn recovery_preserves_preexisting_artifacts_and_is_repeatable() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    crate::database::test_support::delete_database();
    crate::test_support::init_test_keychain();
    crate::test_support::shared_mock_keychain().reset();

    let metadata = WalletMetadata::preview_new();
    let validated = ValidatedRestoreWalletId::validate(&metadata.id).unwrap();
    let paths = crate::bdk_store::BdkStore::wallet_store_artifact_paths(&metadata.id);
    let preexisting = paths[0].clone();
    let created = paths[1].clone();
    fs::write(&preexisting, b"pre-existing").unwrap();
    let initial = RestoreArtifactSnapshot::capture(&validated).unwrap();
    let marker_path = write_test_marker(&metadata.id, &initial, RestoreMarkerPhase::Writing);
    fs::write(&created, b"created by interrupted restore").unwrap();

    recover_restore_markers().unwrap();
    assert!(preexisting.exists());
    assert!(!created.exists());
    assert!(!marker_path.exists());
    recover_restore_markers().unwrap();

    fs::remove_file(preexisting).unwrap();
}

#[test]
fn recovery_removes_marker_and_unreadable_restore_created_keychain_entry() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    crate::database::test_support::delete_database();
    crate::test_support::init_test_keychain();
    crate::test_support::shared_mock_keychain().reset();

    let mut metadata = WalletMetadata::preview_new();
    metadata.id = WalletId::preview_new_random();
    let validated = ValidatedRestoreWalletId::validate(&metadata.id).unwrap();
    let initial = RestoreArtifactSnapshot::capture(&validated).unwrap();
    let marker_path = write_test_marker(&metadata.id, &initial, RestoreMarkerPhase::Writing);
    let secret_key = format!("{}::wallet_mnemonic", metadata.id);
    crate::test_support::shared_mock_keychain()
        .set_entries(vec![(secret_key.as_str(), "unreadable encrypted secret")]);

    assert!(Keychain::global().get_wallet_secret(&metadata.id).is_err());
    recover_restore_markers().unwrap();

    assert!(!marker_path.exists());
    assert!(!Keychain::global().wallet_items_exist(&metadata.id));
}

#[test]
fn recovery_removes_unreadable_restore_created_entry_and_preserves_xpub() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    crate::database::test_support::delete_database();
    crate::test_support::init_test_keychain();
    crate::test_support::shared_mock_keychain().reset();

    let mut metadata = WalletMetadata::preview_new();
    metadata.id = WalletId::preview_new_random();
    let xpub = bitcoin::bip32::Xpub::from_str(
        "xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM",
    )
    .unwrap();
    Keychain::global().save_wallet_xpub(&metadata.id, xpub).unwrap();

    let validated = ValidatedRestoreWalletId::validate(&metadata.id).unwrap();
    let initial = RestoreArtifactSnapshot::capture(&validated).unwrap();
    let marker_path = write_test_marker(&metadata.id, &initial, RestoreMarkerPhase::Writing);
    let xpub_key = format!("{}::wallet_xpub", metadata.id);
    let xpub_value = xpub.to_string();
    let secret_key = format!("{}::wallet_mnemonic", metadata.id);
    crate::test_support::shared_mock_keychain().set_entries(vec![
        (xpub_key.as_str(), xpub_value.as_str()),
        (secret_key.as_str(), "unreadable encrypted secret"),
    ]);

    recover_restore_markers().unwrap();
    assert!(!marker_path.exists());
    assert_eq!(Keychain::global().get_wallet_xpub(&metadata.id).unwrap(), Some(xpub));
    assert!(Keychain::global().get_wallet_secret(&metadata.id).unwrap().is_none());

    assert!(Keychain::global().delete_wallet_xpub(&metadata.id));
}

#[test]
fn recovery_does_not_broad_delete_inconsistent_keychain_snapshot() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    crate::database::test_support::delete_database();
    crate::test_support::init_test_keychain();
    crate::test_support::shared_mock_keychain().reset();

    let mut metadata = WalletMetadata::preview_new();
    metadata.id = WalletId::preview_new_random();
    let initial =
        RestoreArtifactSnapshot { keychain_items: true, ..RestoreArtifactSnapshot::default() };
    let marker_path = write_test_marker(&metadata.id, &initial, RestoreMarkerPhase::Writing);
    let secret_key = format!("{}::wallet_mnemonic", metadata.id);
    crate::test_support::shared_mock_keychain()
        .set_entries(vec![(secret_key.as_str(), "unreadable encrypted secret")]);

    assert!(recover_restore_markers().is_err());
    assert!(marker_path.exists());
    assert!(Keychain::global().get_wallet_secret(&metadata.id).is_err());

    crate::test_support::shared_mock_keychain().reset();
    remove_marker(&marker_path).unwrap();
}

#[test]
fn recovery_does_not_selectively_delete_without_approved_keychain_fingerprints() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    crate::database::test_support::delete_database();
    crate::test_support::init_test_keychain();
    crate::test_support::shared_mock_keychain().reset();

    let mut metadata = WalletMetadata::preview_new();
    metadata.id = WalletId::preview_new_random();
    let initial =
        RestoreArtifactSnapshot { keychain_items: true, ..RestoreArtifactSnapshot::default() };
    let marker_path = write_test_marker(&metadata.id, &initial, RestoreMarkerPhase::Writing);
    let xpub = bitcoin::bip32::Xpub::from_str(
        "xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM",
    )
    .unwrap();
    Keychain::global().save_wallet_xpub(&metadata.id, xpub).unwrap();

    assert!(recover_restore_markers().is_err());
    assert!(marker_path.exists());
    assert_eq!(Keychain::global().get_wallet_xpub(&metadata.id).unwrap(), Some(xpub));

    crate::test_support::shared_mock_keychain().reset();
    remove_marker(&marker_path).unwrap();
}

#[test]
fn rollback_reports_a_changed_keychain_kind_once() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    crate::database::test_support::delete_database();
    crate::test_support::init_test_keychain();
    crate::test_support::shared_mock_keychain().reset();

    let metadata = WalletMetadata::preview_new();
    let original = bitcoin::bip32::Xpub::from_str(
        "xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM",
    )
    .unwrap();
    let changed = bitcoin::bip32::Xpub::from_str(
        "xpub661MyMwAqRbcFFr2SGY3dUn7g8P9VKNZdKWL2Z2pZMEkBWH2D1KTcwTn7keZQCaScCx7BUDjHFJJHnzBvDgUFgNjYsQTRvo7LWfYEtt78Pb",
    )
    .unwrap();
    Keychain::global().save_wallet_xpub(&metadata.id, original).unwrap();

    let validated = ValidatedRestoreWalletId::validate(&metadata.id).unwrap();
    let initial = RestoreArtifactSnapshot::capture(&validated).unwrap();
    let key = format!("{}::wallet_xpub", metadata.id);
    let changed_value = changed.to_string();
    crate::test_support::shared_mock_keychain()
        .set_entries(vec![(key.as_str(), changed_value.as_str())]);

    let mut failures = Vec::new();
    rollback_keychain(&metadata.id, &initial, &mut failures);

    assert_eq!(failures.len(), 1);
    assert!(failures[0].contains("pre-existing keychain xpub changed"));

    crate::test_support::shared_mock_keychain().reset();
}

#[test]
fn recovery_retains_marker_when_owned_cleanup_fails() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    crate::database::test_support::delete_database();
    crate::test_support::init_test_keychain();
    crate::test_support::shared_mock_keychain().reset();

    let metadata = WalletMetadata::preview_new();
    let path = crate::bdk_store::BdkStore::wallet_store_artifact_paths(&metadata.id)[1].clone();
    fs::create_dir(&path).unwrap();
    let marker_path = write_test_marker(
        &metadata.id,
        &RestoreArtifactSnapshot::default(),
        RestoreMarkerPhase::Writing,
    );

    assert!(recover_restore_markers().is_err());
    assert!(marker_path.exists());

    fs::remove_dir(&path).unwrap();
    recover_restore_markers().unwrap();
    assert!(!marker_path.exists());
}

#[test]
fn recovery_removes_marker_after_metadata_commit_without_cleaning_artifacts() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    crate::database::test_support::delete_database();
    crate::app::reconcile::test_support::init_noop_updater();
    crate::test_support::init_test_keychain();
    crate::test_support::shared_mock_keychain().reset();

    let metadata = WalletMetadata::preview_new();
    let path = crate::bdk_store::BdkStore::wallet_store_artifact_paths(&metadata.id)[1].clone();
    fs::write(&path, b"committed artifact").unwrap();
    let validated = ValidatedRestoreWalletId::validate(&metadata.id).unwrap();
    let initial = RestoreArtifactSnapshot::capture(&validated).unwrap();
    let marker_path = write_test_marker(&metadata.id, &initial, RestoreMarkerPhase::Writing);
    Database::global().wallets.save_restored_wallet_metadata(metadata.clone()).unwrap();

    recover_restore_markers().unwrap();
    assert!(!marker_path.exists());
    assert!(path.exists());

    fs::remove_file(path).unwrap();
}

#[test]
fn durable_wallet_lock_blocks_a_concurrent_lease() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    crate::database::test_support::delete_database();
    crate::test_support::init_test_keychain();
    crate::test_support::shared_mock_keychain().reset();

    let mut metadata = WalletMetadata::preview_new();
    metadata.id = WalletId::preview_new_random();
    let validated = ValidatedRestoreWalletId::validate(&metadata.id).unwrap();
    let lease = WalletRestoreLease::acquire(&metadata).unwrap();

    assert!(RestoreFileLock::is_held(&validated));
    assert!(matches!(
        WalletRestoreLease::acquire(&metadata),
        Err(BackupError::WalletIdOccupied(id)) if id == metadata.id
    ));

    drop(lease);
    assert!(!RestoreFileLock::is_held(&validated));
    assert!(!lock_path(&validated).exists(), "the lease must not leak its lock file");
    assert!(WalletRestoreLease::acquire(&metadata).is_ok());
}

#[test]
fn marker_recovery_defers_while_a_restore_holds_the_lease() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    crate::database::test_support::delete_database();
    crate::test_support::init_test_keychain();
    crate::test_support::shared_mock_keychain().reset();

    let mut metadata = WalletMetadata::preview_new();
    metadata.id = WalletId::preview_new_random();
    let lease = WalletRestoreLease::acquire(&metadata).unwrap();
    let marker_path = write_test_marker(
        &metadata.id,
        &RestoreArtifactSnapshot::default(),
        RestoreMarkerPhase::Writing,
    );

    recover_restore_markers().unwrap();
    assert!(marker_path.exists(), "recovery must defer, not fail, while a restore is running");

    drop(lease);
    recover_restore_markers().unwrap();
    assert!(!marker_path.exists());
}

#[test]
fn interrupted_cleanup_resumes_from_durable_approval() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    crate::database::test_support::delete_database();
    crate::test_support::init_test_keychain();
    crate::test_support::shared_mock_keychain().reset();

    let mut metadata = WalletMetadata::preview_new();
    metadata.id = WalletId::preview_new_random();
    let path = crate::bdk_store::BdkStore::wallet_store_artifact_paths(&metadata.id)[1].clone();
    fs::write(&path, b"approved artifact").unwrap();
    let validated = ValidatedRestoreWalletId::validate(&metadata.id).unwrap();
    let initial = RestoreArtifactSnapshot::capture(&validated).unwrap();
    let marker_path =
        write_test_marker(&metadata.id, &initial, RestoreMarkerPhase::CleanupInProgress);

    recover_restore_markers().unwrap();

    assert!(!path.exists());
    assert!(!marker_path.exists());
}

#[test]
fn interrupted_cleanup_accepts_remaining_keychain_subset() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    crate::database::test_support::delete_database();
    crate::test_support::init_test_keychain();
    crate::test_support::shared_mock_keychain().reset();

    let mut metadata = WalletMetadata::preview_new();
    metadata.id = WalletId::preview_new_random();
    let mnemonic = bip39::Mnemonic::from_str(
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
    )
    .unwrap();
    let xpub = bitcoin::bip32::Xpub::from_str(
        "xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM",
    )
    .unwrap();
    Keychain::global().save_wallet_key(&metadata.id, mnemonic).unwrap();
    Keychain::global().save_wallet_xpub(&metadata.id, xpub).unwrap();

    let validated = ValidatedRestoreWalletId::validate(&metadata.id).unwrap();
    let initial = RestoreArtifactSnapshot::capture(&validated).unwrap();
    let marker_path =
        write_test_marker(&metadata.id, &initial, RestoreMarkerPhase::CleanupInProgress);
    crate::test_support::shared_mock_keychain().fail_delete_at(3);

    assert!(recover_restore_markers().is_err());
    assert!(marker_path.exists());
    assert!(Keychain::global().get_wallet_secret(&metadata.id).unwrap().is_none());
    assert_eq!(Keychain::global().get_wallet_xpub(&metadata.id).unwrap(), Some(xpub));

    recover_restore_markers().unwrap();
    assert!(!marker_path.exists());
    assert!(!Keychain::global().wallet_items_exist(&metadata.id));
}

#[test]
fn empty_wallet_data_directory_is_not_an_occupied_restore_artifact() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    crate::database::test_support::delete_database();
    crate::test_support::init_test_keychain();
    crate::test_support::shared_mock_keychain().reset();

    let mut metadata = WalletMetadata::preview_new();
    metadata.id = WalletId::preview_new_random();
    let directory = crate::database::wallet_data::wallet_data_directory_path(&metadata.id);
    fs::create_dir_all(&directory).unwrap();

    let id = ValidatedRestoreWalletId::validate(&metadata.id).unwrap();
    let snapshot = RestoreArtifactSnapshot::capture(&id).unwrap();

    assert!(snapshot.wallet_data_paths.contains(&directory));
    assert!(!snapshot.wallet_data_occupied);
    assert!(!snapshot.is_occupied());

    fs::remove_dir(directory).unwrap();
}

#[test]
fn nested_wallet_data_cleanup_preserves_existing_entries() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    crate::database::test_support::delete_database();
    crate::test_support::init_test_keychain();
    crate::test_support::shared_mock_keychain().reset();

    let mut metadata = WalletMetadata::preview_new();
    metadata.id = WalletId::preview_new_random();
    let directory = crate::database::wallet_data::wallet_data_directory_path(&metadata.id);
    let existing = directory.join("nested/deep/existing.redb");
    let created = directory.join("nested/deep/created.redb");
    #[cfg(unix)]
    let external = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    let external_sentinel = external.path().join("sentinel");
    #[cfg(unix)]
    fs::write(&external_sentinel, b"outside wallet data").unwrap();
    if let Some(parent) = existing.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&existing, b"existing").unwrap();
    let validated = ValidatedRestoreWalletId::validate(&metadata.id).unwrap();
    let initial = RestoreArtifactSnapshot::capture(&validated).unwrap();
    let marker_path = write_test_marker(&metadata.id, &initial, RestoreMarkerPhase::Writing);
    fs::write(&created, b"created").unwrap();
    #[cfg(unix)]
    {
        let link = directory.join("nested/external");
        std::os::unix::fs::symlink(external.path(), &link).unwrap();
    }

    recover_restore_markers().unwrap();

    assert!(existing.exists());
    assert!(!created.exists());
    assert!(!marker_path.exists());
    #[cfg(unix)]
    {
        assert!(external_sentinel.exists());
        assert!(fs::symlink_metadata(directory.join("nested/external")).is_err());
    }

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn malformed_marker_is_quarantined_without_blocking_recovery() {
    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    let directory = marker_directory();
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("bad-marker.json");
    let temporary_path = directory.join(".bad-marker.json.tmp");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&temporary_path);
    fs::write(&path, b"not-json").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(directory.join("missing-target"), &temporary_path).unwrap();

    recover_restore_markers().unwrap();

    assert!(!path.exists());
    let quarantine = directory.join(QUARANTINE_DIRECTORY_NAME);
    assert!(fs::read_dir(&quarantine).unwrap().next().is_some());
    fs::remove_dir_all(quarantine).unwrap();
    let _ = fs::remove_file(temporary_path);
}

#[cfg(unix)]
#[test]
fn approval_rejects_symlink_replacement() {
    use std::os::unix::fs::symlink;

    let _guard = crate::test_support::global_state_test_lock().blocking_lock();
    crate::database::test_support::delete_database();
    crate::test_support::init_test_keychain();
    crate::test_support::shared_mock_keychain().reset();

    let mut metadata = WalletMetadata::preview_new();
    metadata.id = WalletId::preview_new_random();
    let artifact = crate::bdk_store::BdkStore::wallet_store_artifact_paths(&metadata.id)[1].clone();
    fs::write(&artifact, b"original").unwrap();
    let validated = ValidatedRestoreWalletId::validate(&metadata.id).unwrap();
    let expected = RestoreArtifactSnapshot::capture(&validated).unwrap();
    let target = artifact.with_extension("replacement");
    fs::write(&target, b"replacement").unwrap();
    fs::remove_file(&artifact).unwrap();
    symlink(&target, &artifact).unwrap();

    assert!(matches!(
        WalletRestoreLease::acquire_for_approval(&metadata, &expected),
        Err(BackupError::ImportApprovalStale(id)) if id == metadata.id
    ));

    fs::remove_file(&artifact).unwrap();
    fs::remove_file(target).unwrap();
}
