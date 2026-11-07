//! Restores a Cloud Backup that the v1.3.0 code wrote, on a new device running this code
//!
//! The fixture in `fixtures/v1_3_0_cloud_backup` is what the v1.3.0 manager uploaded during a
//! fresh enable against the scripted cloud and passkey, with every file at the path 1.3.0 wrote
//! it; its README says how it was made and how to regenerate it

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cove_cspp::backup_data::MASTER_KEY_RECORD_ID;
use serde::Deserialize;

use super::*;
use crate::manager::cloud_backup_manager::PASSKEY_RP_ID;
use crate::network::Network;
use crate::wallet::WalletAddressType;
use crate::wallet::metadata::{HardwareWalletMetadata, WalletColor, WalletId};

#[derive(Debug, Deserialize)]
struct GoldenManifest {
    cove_version: String,
    namespace_id: String,
    files: Vec<GoldenFile>,
    passkey: GoldenPasskey,
    wallets: Vec<GoldenWallet>,
}

#[derive(Debug, Deserialize)]
struct GoldenFile {
    path: String,
    record_id: String,
}

#[derive(Debug, Deserialize)]
struct GoldenPasskey {
    credential_id_hex: String,
    prf_output_hex: String,
    prf_salt_hex: String,
    requests: Vec<GoldenPasskeyRequest>,
}

#[derive(Debug, Deserialize)]
struct GoldenPasskeyRequest {
    operation: String,
    rp_id: String,
}

#[derive(Debug, Deserialize)]
struct GoldenWallet {
    id: String,
    name: String,
    network: Network,
    wallet_mode: WalletMode,
    wallet_type: WalletType,
    color: WalletColor,
    address_type: WalletAddressType,
    fingerprint: Option<String>,
    origin: Option<String>,
    mnemonic: Option<String>,
    xpub: String,
    external_descriptor: String,
    internal_descriptor: String,
    tap_signer_backup_hex: Option<String>,
    labels_jsonl: Option<String>,
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/manager/cloud_backup_manager/ops/tests/fixtures/v1_3_0_cloud_backup")
}

fn load_manifest() -> GoldenManifest {
    let manifest = std::fs::read(fixture_dir().join("manifest.json")).unwrap();
    serde_json::from_slice(&manifest).unwrap()
}

/// Puts every file 1.3.0 wrote into the scripted cloud, listed the way the native providers list
/// a namespace's wallet files
fn load_fixture_into_cloud(globals: &TestGlobals, manifest: &GoldenManifest) {
    let namespace_dir = fixture_dir().join("cloud").join(&manifest.namespace_id);
    let mut wallet_files = Vec::new();

    for file in &manifest.files {
        let bytes = std::fs::read(namespace_dir.join(&file.path)).unwrap();
        if file.record_id == MASTER_KEY_RECORD_ID {
            globals.cloud.set_master_key_backup(manifest.namespace_id.clone(), bytes);
            continue;
        }

        globals.cloud.set_wallet_backup(
            manifest.namespace_id.clone(),
            file.record_id.clone(),
            bytes,
        );
        wallet_files.push(file.path.clone());
    }

    globals.cloud.set_wallet_files(manifest.namespace_id.clone(), wallet_files);
}

/// Removes the on-disk wallet stores of the fixture's wallets, whose ids are fixed, so each run
/// starts on a device that has never held them
fn delete_fixture_wallet_stores(manifest: &GoldenManifest) {
    for wallet in &manifest.wallets {
        crate::wallet::delete_wallet_specific_data(&WalletId::from(wallet.id.clone())).unwrap();
    }
}

async fn assert_wallet_restored(expected: &GoldenWallet) {
    let id = WalletId::from(expected.id.clone());
    let name = expected.name.as_str();
    let restored = Database::global()
        .wallets()
        .get(&id, expected.network, expected.wallet_mode)
        .unwrap()
        .unwrap_or_else(|| panic!("{name} restored into its network and mode"));

    assert_eq!(restored.name, expected.name);
    assert_eq!(restored.network, expected.network, "{name} network");
    assert_eq!(restored.wallet_mode, expected.wallet_mode, "{name} mode");
    assert_eq!(restored.wallet_type, expected.wallet_type, "{name} type");
    assert_eq!(restored.color, expected.color, "{name} color");
    assert_eq!(restored.address_type, expected.address_type, "{name} address type");
    assert_eq!(restored.origin, expected.origin, "{name} origin");
    assert_eq!(
        restored.master_fingerprint.as_ref().map(|fingerprint| fingerprint.as_uppercase()),
        expected.fingerprint,
        "{name} fingerprint"
    );
    assert_eq!(
        restored.hardware_metadata.as_ref().is_some_and(HardwareWalletMetadata::is_tap_signer),
        expected.tap_signer_backup_hex.is_some(),
        "{name} TAPSIGNER metadata"
    );

    let keychain = Keychain::global();
    assert_eq!(
        keychain.get_wallet_key(&id).unwrap().map(|mnemonic| mnemonic.to_string()),
        expected.mnemonic,
        "{name} recovery words"
    );
    assert_eq!(
        keychain.get_tap_signer_backup(&id).unwrap().map(|backup| hex::encode(&*backup)),
        expected.tap_signer_backup_hex,
        "{name} TAPSIGNER backup"
    );

    let (external, internal) = keychain
        .get_public_descriptor(&id)
        .unwrap()
        .unwrap_or_else(|| panic!("{name} public descriptors"));
    assert_eq!(external.to_string(), expected.external_descriptor, "{name} external descriptor");
    assert_eq!(internal.to_string(), expected.internal_descriptor, "{name} internal descriptor");

    assert_eq!(
        keychain.get_wallet_xpub(&id).unwrap().map(|xpub| xpub.to_string()),
        Some(expected.xpub.clone()),
        "{name} xpub"
    );

    let exported = LabelManager::try_new(id).unwrap().export().await.unwrap();
    match &expected.labels_jsonl {
        Some(labels) => assert_labels_restored(name, labels, &exported),
        None => assert!(exported.trim().is_empty(), "{name} has no labels: {exported}"),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn restore_on_new_device_brings_back_every_wallet_a_v1_3_0_backup_holds() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    let manifest = load_manifest();

    assert_eq!(manifest.cove_version, "1.3.0");

    // a new device: no wallets, no Cloud Backup state, and the 1.3.0 backup in the cloud
    reset_cloud_backup_test_state(&manager, globals);
    delete_fixture_wallet_stores(&manifest);
    load_fixture_into_cloud(globals, &manifest);

    let credential_id = hex::decode(&manifest.passkey.credential_id_hex).unwrap();
    let prf_output = hex::decode(&manifest.passkey.prf_output_hex).unwrap();
    let prf_salt = hex::decode(&manifest.passkey.prf_salt_hex).unwrap();
    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: prf_output.clone(),
        credential_id: credential_id.clone(),
    }));
    globals.passkey.set_authenticate_result(Ok(prf_output));

    let report = restore_like_onboarding(&manager).await;

    assert_eq!(report.failed_wallet_errors, Vec::<String>::new(), "wallet restore errors");
    assert_eq!(report.labels_failed_errors, Vec::<String>::new(), "label restore errors");
    assert_eq!(report.wallets_restored as usize, manifest.wallets.len(), "wallets restored");
    assert_eq!(report.wallets_failed, 0, "wallets failed");

    for wallet in &manifest.wallets {
        assert_wallet_restored(wallet).await;
    }

    // the passkey a 1.3.0 install registered still answers: same relying party and PRF salt
    let registered_rp_id = manifest
        .passkey
        .requests
        .iter()
        .find(|request| request.operation == "create")
        .map(|request| request.rp_id.as_str())
        .expect("1.3.0 registered a passkey");
    assert_eq!(PASSKEY_RP_ID, registered_rp_id, "relying party a 1.3.0 passkey is registered for");

    let discovered = globals.passkey.discovered_requests();
    let authenticated = globals.passkey.authenticated_requests();
    assert!(!discovered.is_empty(), "restore asks for a discoverable passkey");
    for request in &discovered {
        assert_eq!(request.rp_id, registered_rp_id, "discovery relying party");
        assert_eq!(request.prf_salt, prf_salt, "discovery PRF salt");
    }

    for request in &authenticated {
        assert_eq!(request.rp_id, registered_rp_id, "authentication relying party");
        assert_eq!(request.credential_id, credential_id, "authentication credential");
        assert_eq!(request.prf_salt, prf_salt, "authentication PRF salt");
    }

    // every file was read at the path 1.3.0 wrote it
    let mut read_paths: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let downloads = globals.cloud.download_requests();
    for download in &downloads {
        assert_eq!(download.namespace, manifest.namespace_id);
        read_paths
            .entry(download.record_id.as_str())
            .or_default()
            .extend(download.relative_paths.iter().map(String::as_str));
    }

    for file in &manifest.files {
        let paths = read_paths.get(file.record_id.as_str()).cloned().unwrap_or_default();
        assert!(paths.contains(&file.path.as_str()), "{} read from {paths:?}", file.path);
    }

    // this device now backs up to the same namespace with the same passkey
    let cloud_keychain = CloudBackupKeychain::global();
    assert_eq!(cloud_keychain.namespace_id().unwrap(), Some(manifest.namespace_id.clone()));
    assert_eq!(cloud_keychain.load_credential_id().unwrap(), Some(credential_id));
    assert_eq!(cloud_keychain.load_prf_salt().unwrap().map(Vec::from), Some(prf_salt));
    assert_eq!(manager.current_status(), CloudBackupStatus::Enabled);

    reset_cloud_backup_test_state(&manager, globals);
    delete_fixture_wallet_stores(&manifest);
}
