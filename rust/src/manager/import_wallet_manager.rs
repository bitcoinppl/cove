use crate::database::global_config::SelectedWalletTarget;
use bip39::{Language, Mnemonic};
use cove_device::keychain::WalletSecret;
use cove_util::result_ext::ResultExt as _;

use crate::{
    database::{self, Database},
    keychain::{Keychain, KeychainError},
    manager::cloud_backup_manager::CLOUD_BACKUP_MANAGER,
    network::Network,
    wallet::{
        Wallet,
        fingerprint::Fingerprint,
        metadata::{WalletId, WalletMetadata, WalletMode},
        secret_upgrade::{ExistingWalletSecretUpgrade, SecretUpgradeError, SecretUpgradeSource},
    },
    wallet_secret::WalletSecretExt as _,
};

#[derive(Clone, Debug, uniffi::Object)]
pub struct RustImportWalletManager;

#[derive(Debug, Clone, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
pub enum ImportWalletError {
    #[error("failed to import wallet: {0}")]
    WalletImportError(String),

    #[error("invalid word group: {0}")]
    InvalidWordGroup(String),

    #[error("failed to save wallet to keychain: {0}")]
    Keychain(#[from] KeychainError),

    #[error("wallet already exists")]
    WalletAlreadyExists(WalletId),

    #[error("wallet metadata missing for existing wallet")]
    MissingMetadata(WalletId),

    #[error("failed to save wallet: {0}")]
    Database(#[from] database::Error),

    #[error("failed to create wallet: {0}")]
    BdkError(String),

    #[error("multiple existing wallets could match the incoming secret")]
    WalletIdentityCollision,

    #[error("failed to verify existing wallet public identity: {0}")]
    WalletIdentity(String),

    #[error("failed to roll back an incomplete existing-wallet upgrade: {0}")]
    UpgradeRollback(String),
}

pub type Error = ImportWalletError;

impl From<SecretUpgradeError> for ImportWalletError {
    fn from(error: SecretUpgradeError) -> Self {
        match error {
            SecretUpgradeError::AlreadyHoldsSecret(id)
            | SecretUpgradeError::UnreadableSecret(id) => Self::WalletAlreadyExists(id),
            SecretUpgradeError::IdentityCollision => Self::WalletIdentityCollision,
            SecretUpgradeError::Identity(error) => Self::WalletIdentity(error),
            SecretUpgradeError::Keychain(error) => Self::Keychain(error),
            SecretUpgradeError::Database(error) => Self::Database(error),
            SecretUpgradeError::Lifecycle(error) => Self::WalletImportError(error),
            SecretUpgradeError::Rollback(error) => Self::UpgradeRollback(error),
        }
    }
}

#[derive(Clone, Copy)]
enum ImportedWalletDefaultName {
    Numbered,
    KeyTeleportFingerprint,
}

impl ImportedWalletDefaultName {
    fn resolve(self, fingerprint: Fingerprint, wallet_count: u16) -> String {
        match self {
            Self::Numbered => format!("Wallet {}", wallet_count + 1),
            Self::KeyTeleportFingerprint => {
                format!("KeyTeleport {}", fingerprint.as_uppercase())
            }
        }
    }
}

#[uniffi::export]
impl RustImportWalletManager {
    #[uniffi::constructor]
    pub fn new() -> Self {
        Self
    }

    /// Import wallet view from entered words
    #[uniffi::method]
    pub fn import_wallet(&self, entered_words: Vec<Vec<String>>) -> Result<WalletMetadata, Error> {
        let words = entered_words.into_iter().flatten().collect::<Vec<String>>().join(" ");

        let mnemonic = Mnemonic::parse_in_normalized(Language::English, &words)
            .map_err_str(ImportWalletError::InvalidWordGroup)?;

        let SelectedWalletTarget { network, mode } =
            Database::global().global_config.wallet_target();

        import_mnemonic_with_target(mnemonic, network, mode)
    }
}

pub(crate) fn import_mnemonic_with_target(
    mnemonic: Mnemonic,
    network: Network,
    mode: WalletMode,
) -> Result<WalletMetadata, Error> {
    import_wallet_secret_with_target(mnemonic.into(), network, mode)
}

pub(crate) fn import_wallet_secret_with_target(
    secret: WalletSecret,
    network: Network,
    mode: WalletMode,
) -> Result<WalletMetadata, Error> {
    import_wallet_secret_with_default_name(
        secret,
        network,
        mode,
        ImportedWalletDefaultName::Numbered,
    )
}

pub(crate) fn import_key_teleport_wallet_secret_with_target(
    secret: WalletSecret,
    network: Network,
    mode: WalletMode,
) -> Result<WalletMetadata, Error> {
    import_wallet_secret_with_default_name(
        secret,
        network,
        mode,
        ImportedWalletDefaultName::KeyTeleportFingerprint,
    )
}

fn import_wallet_secret_with_default_name(
    secret: WalletSecret,
    network: Network,
    mode: WalletMode,
    default_name: ImportedWalletDefaultName,
) -> Result<WalletMetadata, Error> {
    let fingerprint: Fingerprint = secret.xpub(network).fingerprint().into();
    let database = Database::global();
    let keychain = Keychain::global();

    let upgrade = ExistingWalletSecretUpgrade::find(&database, keychain, &secret, network, mode)?;
    let Some(upgrade) = upgrade else {
        return create_new_wallet(secret, network, mode, fingerprint, default_name, &database);
    };

    let metadata = upgrade.apply(&database, keychain, SecretUpgradeSource::Import)?;
    CLOUD_BACKUP_MANAGER.handle_wallet_backup_change_and_reverify(metadata.id.clone());

    Ok(metadata)
}

fn create_new_wallet(
    secret: WalletSecret,
    network: Network,
    mode: WalletMode,
    fingerprint: Fingerprint,
    default_name: ImportedWalletDefaultName,
    database: &Database,
) -> Result<WalletMetadata, Error> {
    let _construction = crate::wallet_lifecycle::WalletLifecycleCoordinator::global()
        .begin_unscoped_construction()
        .map_err_str(ImportWalletError::WalletImportError)?;
    let number_of_wallets = database.wallets.len(network, mode)?;
    let name = default_name.resolve(fingerprint, number_of_wallets);
    let mut metadata = match &secret {
        WalletSecret::Mnemonic(_) => {
            WalletMetadata::new_imported_from_mnemonic(name, network, fingerprint)
        }
        WalletSecret::Xpriv(_) => {
            WalletMetadata::new_imported_from_xpriv(name, network, fingerprint)
        }
    };

    metadata.wallet_mode = mode;

    match secret {
        WalletSecret::Mnemonic(mnemonic) => {
            Wallet::try_new_persisted_and_selected(metadata.clone(), mnemonic, None)
        }
        WalletSecret::Xpriv(xpriv) => {
            Wallet::try_new_persisted_xpriv_and_selected(metadata.clone(), xpriv)
        }
    }
    .map_err_str(ImportWalletError::WalletImportError)?;
    CLOUD_BACKUP_MANAGER.handle_wallet_set_change();

    Ok(metadata)
}

#[cfg(test)]
mod tests {
    use std::{
        str::FromStr as _,
        sync::{Arc, Once},
    };

    use super::*;
    use crate::wallet::{WalletAddressType, metadata::WalletType};
    use bdk_wallet::bitcoin::bip32::Xpriv;
    use cove_cspp::CsppStore as _;
    use cove_device::keychain::WalletXprv;

    fn init_globals() {
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            crate::test_support::ensure_tokio_runtime();
            crate::database::test_support::init_test_database();
            crate::test_support::init_test_keychain();
        });
    }

    fn xpriv_secret(seed_byte: u8) -> WalletSecret {
        let xpriv =
            Xpriv::new_master(bdk_wallet::bitcoin::Network::Bitcoin, &[seed_byte; 32]).unwrap();

        WalletSecret::Xpriv(WalletXprv::try_from(xpriv).unwrap())
    }

    fn save_watch_only_wallet(
        public_material: &WalletSecret,
        fingerprint: Fingerprint,
        network: Network,
        mode: WalletMode,
    ) -> WalletMetadata {
        let descriptors =
            public_material.clone().into_descriptors(network, WalletAddressType::NativeSegwit);
        let id = WalletId::new();
        let mut metadata = WalletMetadata::preview_new();
        metadata.id = id.clone();
        metadata.name = format!("Watch-only {id}");
        metadata.master_fingerprint = Some(Arc::new(fingerprint));
        metadata.origin = descriptors.origin().ok();
        metadata.network = network;
        metadata.wallet_mode = mode;
        metadata.address_type = WalletAddressType::NativeSegwit;
        metadata.wallet_type = WalletType::WatchOnly;
        metadata.verified = false;

        Keychain::global().save_wallet_xpub(&id, public_material.xpub(network)).unwrap();
        Keychain::global()
            .save_public_descriptor(
                &id,
                descriptors.external.extended_descriptor,
                descriptors.internal.extended_descriptor,
            )
            .unwrap();
        Database::global().wallets.save_new_wallet_metadata(metadata.clone()).unwrap();

        metadata
    }

    fn save_degraded_watch_only_wallet(
        fingerprint: Fingerprint,
        network: Network,
        mode: WalletMode,
    ) -> WalletMetadata {
        let id = WalletId::new();
        let mut metadata = WalletMetadata::preview_new();
        metadata.id = id.clone();
        metadata.name = format!("Degraded watch-only {id}");
        metadata.master_fingerprint = Some(Arc::new(fingerprint));
        metadata.network = network;
        metadata.wallet_mode = mode;
        metadata.address_type = WalletAddressType::NativeSegwit;
        metadata.wallet_type = WalletType::WatchOnly;
        metadata.verified = false;

        Database::global().wallets.save_new_wallet_metadata(metadata.clone()).unwrap();

        metadata
    }

    #[test]
    fn import_mnemonic_uses_explicit_target_scope() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();
        Database::global().global_config.set_selected_network(Network::Bitcoin).unwrap();
        Database::global().global_config.set_main_mode().unwrap();

        let mnemonic = Mnemonic::from_str(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap();

        let metadata =
            import_mnemonic_with_target(mnemonic, Network::Signet, WalletMode::Decoy).unwrap();

        assert_eq!(metadata.network, Network::Signet);
        assert_eq!(metadata.wallet_mode, WalletMode::Decoy);
        assert!(
            Database::global()
                .wallets
                .get(&metadata.id, Network::Signet, WalletMode::Decoy)
                .unwrap()
                .is_some()
        );
        assert!(
            Database::global()
                .wallets
                .get(&metadata.id, Network::Bitcoin, WalletMode::Main)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn import_xpriv_uses_explicit_target_scope() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();
        Database::global().global_config.set_selected_network(Network::Bitcoin).unwrap();
        Database::global().global_config.set_main_mode().unwrap();

        let xpriv = Xpriv::new_master(bdk_wallet::bitcoin::Network::Bitcoin, &[11; 32]).unwrap();
        let metadata = import_wallet_secret_with_target(
            WalletSecret::Xpriv(WalletXprv::try_from(xpriv).unwrap()),
            Network::Signet,
            WalletMode::Decoy,
        )
        .unwrap();

        assert_eq!(metadata.network, Network::Signet);
        assert_eq!(metadata.wallet_mode, WalletMode::Decoy);
        assert!(
            Keychain::global()
                .get_wallet_secret(&metadata.id)
                .unwrap()
                .is_some_and(|secret| secret.as_xprv().is_some())
        );
        assert!(
            Database::global()
                .wallets
                .get(&metadata.id, Network::Signet, WalletMode::Decoy)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn key_teleport_import_uses_fingerprint_as_default_name() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();

        let xpriv = Xpriv::new_master(bdk_wallet::bitcoin::Network::Bitcoin, &[12; 32]).unwrap();
        let secret = WalletSecret::Xpriv(WalletXprv::try_from(xpriv).unwrap());
        let fingerprint: Fingerprint = secret.xpub(Network::Signet).fingerprint().into();
        let metadata = import_key_teleport_wallet_secret_with_target(
            secret,
            Network::Signet,
            WalletMode::Decoy,
        )
        .unwrap();

        assert_eq!(metadata.name, format!("KeyTeleport {}", fingerprint.as_uppercase()));
    }

    #[test]
    fn same_fingerprint_with_different_identity_imports_as_a_distinct_wallet() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();

        let incoming = xpriv_secret(31);
        let colliding_public_material = xpriv_secret(32);
        let fingerprint: Fingerprint = incoming.xpub(Network::Signet).fingerprint().into();
        let existing = save_watch_only_wallet(
            &colliding_public_material,
            fingerprint,
            Network::Signet,
            WalletMode::Main,
        );

        let imported =
            import_wallet_secret_with_target(incoming, Network::Signet, WalletMode::Main).unwrap();

        assert_ne!(imported.id, existing.id);
        assert_eq!(imported.wallet_type, WalletType::Hot);
        assert!(Keychain::global().get_wallet_secret(&imported.id).unwrap().is_some());
        assert!(Keychain::global().get_wallet_secret(&existing.id).unwrap().is_none());
        assert_eq!(
            Database::global()
                .wallets
                .get(&existing.id, Network::Signet, WalletMode::Main)
                .unwrap()
                .unwrap()
                .wallet_type,
            WalletType::WatchOnly
        );
    }

    #[test]
    fn existing_wallet_upgrade_preserves_public_material_and_origin() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();

        let incoming = xpriv_secret(33);
        let fingerprint: Fingerprint = incoming.xpub(Network::Signet).fingerprint().into();
        let existing =
            save_watch_only_wallet(&incoming, fingerprint, Network::Signet, WalletMode::Decoy);
        let expected_descriptors =
            Keychain::global().get_public_descriptor(&existing.id).unwrap().unwrap();
        let expected_xpub = Keychain::global().get_wallet_xpub(&existing.id).unwrap().unwrap();
        let expected_origin = existing.origin.clone();

        let upgraded =
            import_wallet_secret_with_target(incoming, Network::Signet, WalletMode::Decoy).unwrap();

        assert_eq!(upgraded.wallet_type, WalletType::Hot);
        assert_eq!(upgraded.origin, expected_origin);
        assert_eq!(
            Keychain::global().get_public_descriptor(&existing.id).unwrap().unwrap(),
            expected_descriptors
        );
        assert_eq!(
            Keychain::global().get_wallet_xpub(&existing.id).unwrap().unwrap(),
            expected_xpub
        );
    }

    #[test]
    fn existing_wallet_upgrade_repairs_one_wallet_missing_public_material() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();

        let incoming = xpriv_secret(40);
        let fingerprint: Fingerprint = incoming.xpub(Network::Signet).fingerprint().into();
        let degraded =
            save_degraded_watch_only_wallet(fingerprint, Network::Signet, WalletMode::Decoy);

        let upgraded =
            import_wallet_secret_with_target(incoming, Network::Signet, WalletMode::Decoy).unwrap();

        assert_eq!(upgraded.id, degraded.id);
        assert_eq!(upgraded.wallet_type, WalletType::Hot);
        assert!(Keychain::global().get_wallet_secret(&degraded.id).unwrap().is_some());
    }

    #[test]
    fn exact_public_identity_match_wins_over_degraded_fingerprint_match() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();

        let incoming = xpriv_secret(41);
        let fingerprint: Fingerprint = incoming.xpub(Network::Signet).fingerprint().into();
        let degraded =
            save_degraded_watch_only_wallet(fingerprint, Network::Signet, WalletMode::Main);
        let exact =
            save_watch_only_wallet(&incoming, fingerprint, Network::Signet, WalletMode::Main);

        let upgraded =
            import_wallet_secret_with_target(incoming, Network::Signet, WalletMode::Main).unwrap();

        assert_eq!(upgraded.id, exact.id);
        assert!(Keychain::global().get_wallet_secret(&exact.id).unwrap().is_some());
        assert!(Keychain::global().get_wallet_secret(&degraded.id).unwrap().is_none());
    }

    #[test]
    fn multiple_degraded_fingerprint_matches_are_ambiguous() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();

        let incoming = xpriv_secret(42);
        let fingerprint: Fingerprint = incoming.xpub(Network::Signet).fingerprint().into();
        let first =
            save_degraded_watch_only_wallet(fingerprint, Network::Signet, WalletMode::Decoy);
        let second =
            save_degraded_watch_only_wallet(fingerprint, Network::Signet, WalletMode::Decoy);

        assert!(matches!(
            import_wallet_secret_with_target(incoming, Network::Signet, WalletMode::Decoy),
            Err(ImportWalletError::WalletIdentityCollision)
        ));
        assert!(Keychain::global().get_wallet_secret(&first.id).unwrap().is_none());
        assert!(Keychain::global().get_wallet_secret(&second.id).unwrap().is_none());
    }

    #[test]
    fn existing_wallet_upgrade_retries_after_secret_was_already_saved() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();

        let incoming = xpriv_secret(34);
        let expected = incoming.clone();
        let fingerprint: Fingerprint = incoming.xpub(Network::Signet).fingerprint().into();
        let existing =
            save_watch_only_wallet(&incoming, fingerprint, Network::Signet, WalletMode::Main);
        Keychain::global().save_wallet_secret(&existing.id, incoming.clone()).unwrap();

        let upgraded =
            import_wallet_secret_with_target(incoming, Network::Signet, WalletMode::Main).unwrap();

        assert_eq!(upgraded.id, existing.id);
        assert_eq!(upgraded.wallet_type, WalletType::Hot);
        assert_eq!(Keychain::global().get_wallet_secret(&existing.id).unwrap(), Some(expected));
    }

    #[test]
    fn existing_wallet_upgrade_repairs_a_lone_orphaned_secret_entry() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();

        let incoming = xpriv_secret(38);
        let expected = incoming.clone();
        let fingerprint: Fingerprint = incoming.xpub(Network::Signet).fingerprint().into();
        let existing =
            save_watch_only_wallet(&incoming, fingerprint, Network::Signet, WalletMode::Main);
        Keychain::global()
            .save(format!("{}::wallet_mnemonic", existing.id), "orphan".to_string())
            .unwrap();

        let upgraded =
            import_wallet_secret_with_target(incoming, Network::Signet, WalletMode::Main).unwrap();

        assert_eq!(upgraded.id, existing.id);
        assert_eq!(upgraded.wallet_type, WalletType::Hot);
        assert_eq!(Keychain::global().get_wallet_secret(&existing.id).unwrap(), Some(expected));
    }

    #[test]
    fn existing_hot_wallet_with_unreadable_secret_reports_a_duplicate() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();

        let incoming = xpriv_secret(39);
        let fingerprint: Fingerprint = incoming.xpub(Network::Signet).fingerprint().into();
        let mut existing =
            save_watch_only_wallet(&incoming, fingerprint, Network::Signet, WalletMode::Main);
        existing.wallet_type = WalletType::Hot;
        Database::global().wallets.update_wallet_metadata(existing.clone()).unwrap();

        let secret_key = format!("{}::wallet_mnemonic", existing.id);
        let cryptor_key = format!("{}::wallet_mnemonic_encryption_key_and_nonce", existing.id);
        Keychain::global().save(secret_key.clone(), "garbage-secret".to_string()).unwrap();
        Keychain::global().save(cryptor_key.clone(), "garbage-cryptor".to_string()).unwrap();

        let result = import_wallet_secret_with_target(incoming, Network::Signet, WalletMode::Main);

        assert!(matches!(result, Err(ImportWalletError::WalletAlreadyExists(_))));
        assert_eq!(Keychain::global().get(secret_key), Some("garbage-secret".to_string()));
        assert_eq!(Keychain::global().get(cryptor_key), Some("garbage-cryptor".to_string()));

        assert!(Keychain::global().delete_wallet_items(&existing.id));
    }

    #[test]
    fn existing_wallet_upgrade_rejects_multiple_exact_matches() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();

        let incoming = xpriv_secret(35);
        let fingerprint: Fingerprint = incoming.xpub(Network::Signet).fingerprint().into();
        let first =
            save_watch_only_wallet(&incoming, fingerprint, Network::Signet, WalletMode::Decoy);
        let second =
            save_watch_only_wallet(&incoming, fingerprint, Network::Signet, WalletMode::Decoy);

        assert!(matches!(
            import_wallet_secret_with_target(incoming, Network::Signet, WalletMode::Decoy),
            Err(ImportWalletError::WalletIdentityCollision)
        ));
        assert!(Keychain::global().get_wallet_secret(&first.id).unwrap().is_none());
        assert!(Keychain::global().get_wallet_secret(&second.id).unwrap().is_none());
    }
}
