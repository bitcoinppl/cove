//! Generators for the private-key and duplicate wallets that the scripted Cloud Backup UI test
//! hosts restore
//!
//! For the private-key wallet, run
//! `cargo test --lib print_ui_test_xprv_wallet_fixture -- --ignored --nocapture` and paste the
//! printed record into the iOS `CloudBackupFixture.json` and the Android
//! `ScriptedCloudBackupFixture`. The wallet is encrypted under the master key already in those
//! fixtures, so the scripted passkey unlocks it

use base64::Engine as _;
use cove_cspp::backup_data::remote_payload::RemotePayloadMetadata;
use cove_cspp::backup_data::{EncryptedMasterKeyBackup, wallet_filename_from_record_id};
use cove_cspp::master_key_crypto::decrypt_master_key;
use cove_cspp::wallet_crypto::encrypt_wallet_entry_with_remote_metadata;
use cove_device::keychain::WalletXprv;
use cove_types::network::Network;

use super::*;
use crate::manager::import_wallet_manager::import_wallet_secret_with_target;
use crate::wallet::metadata::WalletMode as LocalWalletMode;
use cove_device::keychain::WalletSecret as LocalWalletSecret;

/// BIP32 test vector 1 master key, a published test key that must never hold funds
pub(crate) const UI_TEST_XPRV: &str = "xprv9s21ZrQH143K3QTDL4LXw2F7HEK3wJUD2nW2nRk4stbPy6cq3jPPqjiChkVvvNKmPGJxWUtg6LnF5kejMRNNU3TGtRBeJgk33yuGBxrMPHi";

const UI_TEST_XPRV_WALLET_NAME: &str = "Fixture Private Key Wallet";

#[tokio::test(flavor = "current_thread")]
#[ignore = "prints a fixture for the UI test hosts"]
async fn print_ui_test_xprv_wallet_fixture() {
    let _guard = async_test_lock().lock().await;
    let _globals = test_globals();

    let fixture_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../ios/CoveCloudBackupUITestHost/CloudBackupFixture.json"
    );
    let fixture: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixture_path).unwrap()).unwrap();
    let base64 = base64::engine::general_purpose::STANDARD;

    let master: EncryptedMasterKeyBackup =
        serde_json::from_slice(&base64.decode(fixture["master_base64"].as_str().unwrap()).unwrap())
            .unwrap();
    let prf_key: [u8; 32] =
        hex::decode(fixture["prf_output_hex"].as_str().unwrap()).unwrap().try_into().unwrap();
    let master_key = decrypt_master_key(&master, &prf_key).unwrap();

    let secret = LocalWalletSecret::Xpriv(WalletXprv::parse(UI_TEST_XPRV).unwrap());
    let imported =
        import_wallet_secret_with_target(secret, Network::Bitcoin, LocalWalletMode::Main).unwrap();

    // rename the stored row, since the import returns metadata from before the wallet build
    // filled in its origin, and a backup without one fails to load after restore
    let database = crate::database::Database::global();
    let mut metadata = database
        .wallets
        .get(&imported.id, Network::Bitcoin, LocalWalletMode::Main)
        .unwrap()
        .unwrap();
    assert!(metadata.origin.is_some(), "the stored wallet should have its origin");
    metadata.name = UI_TEST_XPRV_WALLET_NAME.to_string();

    // the test vector key has public mainnet history, so an unfinished address-type discovery
    // would pop a wallet chooser over the screens under test whenever the network answers
    metadata.discovery_state = crate::wallet::metadata::DiscoveryState::ChoseAdressType;
    database.wallets().update_wallet_metadata(metadata.clone()).unwrap();

    let namespace = fixture["namespace"].as_str().unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
    let mut prepared = crate::manager::cloud_backup_manager::wallets::prepare_wallet_backup(
        &metadata,
        metadata.wallet_mode,
    )
    .await
    .unwrap();
    prepared.entry.content_revision_hash = "ui-fixture-xprv-revision".to_string();
    prepared.entry.updated_at = 4;

    let critical_key = zeroize::Zeroizing::new(master_key.critical_data_key());
    let encrypted = encrypt_wallet_entry_with_remote_metadata(
        &prepared.entry,
        &critical_key,
        RemotePayloadMetadata::wallet(namespace, &record_id, metadata.id.as_ref(), 4),
    )
    .unwrap();
    let bytes = serde_json::to_vec(&encrypted).unwrap();

    println!("name: {UI_TEST_XPRV_WALLET_NAME}");
    println!("wallet_id: {}", metadata.id);
    println!("record_id: {record_id}");
    println!("filename: {}", wallet_filename_from_record_id(&record_id));
    println!("base64: {}", base64.encode(bytes));
}

// the UI test hosts serve this record, so a stale paste fails here instead of in a simulator run
#[test]
fn ui_test_xprv_wallet_fixture_decrypts_to_the_test_key() {
    let fixture_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../ios/CoveCloudBackupUITestHost/CloudBackupFixture.json"
    );
    let fixture: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixture_path).unwrap()).unwrap();
    let base64 = base64::engine::general_purpose::STANDARD;

    let master: EncryptedMasterKeyBackup =
        serde_json::from_slice(&base64.decode(fixture["master_base64"].as_str().unwrap()).unwrap())
            .unwrap();
    let prf_key: [u8; 32] =
        hex::decode(fixture["prf_output_hex"].as_str().unwrap()).unwrap().try_into().unwrap();
    let master_key = decrypt_master_key(&master, &prf_key).unwrap();

    let record = &fixture["xprv_wallet"];
    let encrypted: cove_cspp::backup_data::EncryptedWalletBackup =
        serde_json::from_slice(&base64.decode(record["base64"].as_str().unwrap()).unwrap())
            .unwrap();
    let entry = cove_cspp::wallet_crypto::decrypt_wallet_backup(
        &encrypted,
        &master_key.critical_data_key(),
    )
    .unwrap();

    assert!(matches!(&entry.secret, WalletSecret::Xprv(xprv) if xprv == UI_TEST_XPRV));

    // a settled discovery keeps the wallet chooser from covering the screens under test
    assert_eq!(entry.metadata["discovery_state"], serde_json::json!("ChoseAdressType"));

    // a restored wallet without an origin fails the metadata check when the app opens it
    assert!(
        entry.metadata["origin"].as_str().is_some_and(|origin| !origin.is_empty()),
        "the fixture wallet has no origin: {}",
        entry.metadata
    );
    assert_eq!(entry.wallet_id, record["wallet_id"].as_str().unwrap());
    assert_eq!(
        wallet_filename_from_record_id(&cove_cspp::backup_data::wallet_record_id(&entry.wallet_id)),
        record["filename"].as_str().unwrap(),
    );

    // android serves its own copy of the same record
    let android_fixture = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../android/app/src/uiTest/java/org/bitcoinppl/cove/testconfig/ScriptedCloudBackupFixture.kt"
    ))
    .unwrap();
    let android_record_id = cove_cspp::backup_data::wallet_record_id(&entry.wallet_id);
    assert!(
        android_fixture.contains(&format!("\"{android_record_id}\"")),
        "the android fixture names a different xprv wallet record"
    );

    let android_base64: String = android_fixture
        .split("private val xprvWalletWrapper =")
        .nth(1)
        .and_then(|rest| rest.split("\"\"\"").nth(1))
        .unwrap()
        .split_whitespace()
        .collect();
    assert_eq!(android_base64, record["base64"].as_str().unwrap(), "android xprv wrapper is stale");
}

/// Wallet id of a second backup of Fixture Wallet One, as another install of the same wallet
/// would upload it, so the cloud inventory has a record that duplicates a local wallet
const UI_TEST_DUPLICATE_WALLET_ID: &str = "ui-fixture-wallet-1-copy";

fn read_ui_fixture() -> serde_json::Value {
    let fixture_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../ios/CoveCloudBackupUITestHost/CloudBackupFixture.json"
    );

    serde_json::from_slice(&std::fs::read(fixture_path).unwrap()).unwrap()
}

fn ui_fixture_master_key(fixture: &serde_json::Value) -> cove_cspp::master_key::MasterKey {
    let base64 = base64::engine::general_purpose::STANDARD;
    let master: EncryptedMasterKeyBackup =
        serde_json::from_slice(&base64.decode(fixture["master_base64"].as_str().unwrap()).unwrap())
            .unwrap();
    let prf_key: [u8; 32] =
        hex::decode(fixture["prf_output_hex"].as_str().unwrap()).unwrap().try_into().unwrap();

    decrypt_master_key(&master, &prf_key).unwrap()
}

fn decrypt_ui_fixture_wallet(
    fixture: &serde_json::Value,
    record: &serde_json::Value,
) -> cove_cspp::backup_data::WalletEntry {
    let base64 = base64::engine::general_purpose::STANDARD;
    let encrypted: cove_cspp::backup_data::EncryptedWalletBackup =
        serde_json::from_slice(&base64.decode(record["base64"].as_str().unwrap()).unwrap())
            .unwrap();

    cove_cspp::wallet_crypto::decrypt_wallet_backup(
        &encrypted,
        &ui_fixture_master_key(fixture).critical_data_key(),
    )
    .unwrap()
}

/// Fixture Wallet One under the duplicate's wallet id, with nothing else about the wallet changed
fn ui_test_duplicate_wallet_entry(
    fixture: &serde_json::Value,
) -> cove_cspp::backup_data::WalletEntry {
    let mut entry = decrypt_ui_fixture_wallet(fixture, &fixture["wallets"][0]);
    entry.wallet_id = UI_TEST_DUPLICATE_WALLET_ID.to_string();
    entry.metadata["id"] = serde_json::json!(UI_TEST_DUPLICATE_WALLET_ID);
    entry.content_revision_hash = "ui-fixture-duplicate-revision".to_string();
    entry.updated_at = 5;

    entry
}

/// Prints `duplicate_wallet` for the iOS `CloudBackupFixture.json`
///
/// Run with `cargo test --lib print_ui_test_duplicate_wallet_fixture -- --ignored --nocapture`
#[test]
#[ignore = "prints a fixture for the UI test hosts"]
fn print_ui_test_duplicate_wallet_fixture() {
    let fixture = read_ui_fixture();
    let entry = ui_test_duplicate_wallet_entry(&fixture);
    let namespace = fixture["namespace"].as_str().unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(UI_TEST_DUPLICATE_WALLET_ID);
    let critical_key = zeroize::Zeroizing::new(ui_fixture_master_key(&fixture).critical_data_key());
    let encrypted = encrypt_wallet_entry_with_remote_metadata(
        &entry,
        &critical_key,
        RemotePayloadMetadata::wallet(
            namespace,
            &record_id,
            UI_TEST_DUPLICATE_WALLET_ID,
            entry.updated_at,
        ),
    )
    .unwrap();

    let base64 = base64::engine::general_purpose::STANDARD;
    println!("wallet_id: {UI_TEST_DUPLICATE_WALLET_ID}");
    println!("record_id: {record_id}");
    println!("filename: {}", wallet_filename_from_record_id(&record_id));
    println!("base64: {}", base64.encode(serde_json::to_vec(&encrypted).unwrap()));
}

// the restore-all dedup UI test relies on this record being the same wallet as Fixture Wallet One
#[test]
fn ui_test_duplicate_wallet_fixture_is_fixture_wallet_one() {
    let fixture = read_ui_fixture();
    let record = &fixture["duplicate_wallet"];
    let entry = decrypt_ui_fixture_wallet(&fixture, record);

    assert_eq!(entry.wallet_id, UI_TEST_DUPLICATE_WALLET_ID);
    assert_eq!(record["wallet_id"].as_str().unwrap(), UI_TEST_DUPLICATE_WALLET_ID);
    assert_eq!(
        record["record_id"].as_str().unwrap(),
        cove_cspp::backup_data::wallet_record_id(UI_TEST_DUPLICATE_WALLET_ID)
    );
    assert_eq!(
        serde_json::to_value(&entry).unwrap(),
        serde_json::to_value(ui_test_duplicate_wallet_entry(&fixture)).unwrap()
    );

    // android serves its own copy of the same record
    let android_fixture = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../android/app/src/uiTest/java/org/bitcoinppl/cove/testconfig/ScriptedCloudBackupFixture.kt"
    ))
    .unwrap();
    assert!(
        android_fixture.contains(&format!("\"{}\"", record["record_id"].as_str().unwrap())),
        "the android fixture names a different duplicate wallet record"
    );

    let android_base64: String = android_fixture
        .split("private val duplicateWalletWrapper =")
        .nth(1)
        .and_then(|rest| rest.split("\"\"\"").nth(1))
        .unwrap()
        .split_whitespace()
        .collect();
    assert_eq!(
        android_base64,
        record["base64"].as_str().unwrap(),
        "android duplicate wrapper is stale"
    );
}
