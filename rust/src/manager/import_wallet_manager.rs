use bip39::{Language, Mnemonic};
use cove_device::keychain::WalletSecret;
use cove_util::result_ext::ResultExt as _;

use crate::{
    app::reconcile::{Update, Updater},
    database::{self, Database},
    keychain::{Keychain, KeychainError},
    manager::cloud_backup_manager::CLOUD_BACKUP_MANAGER,
    network::Network,
    wallet::{
        Wallet,
        fingerprint::Fingerprint,
        metadata::{WalletId, WalletMetadata, WalletMode, WalletType},
    },
    wallet_identity::{PublicWalletIdentity, PublicWalletIdentityError},
    wallet_secret::WalletSecretExt as _,
};

use tracing::{info, warn};

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

impl From<PublicWalletIdentityError> for ImportWalletError {
    fn from(error: PublicWalletIdentityError) -> Self {
        Self::WalletIdentity(error.to_string())
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

#[derive(Clone, Copy)]
enum UpgradeSecretState {
    Created,
    Preexisting,
}

impl UpgradeSecretState {
    fn rollback(self, keychain: &Keychain, id: &WalletId) -> Result<(), KeychainError> {
        match self {
            Self::Created if !keychain.delete_wallet_secret(id) => Err(KeychainError::Delete),
            Self::Created | Self::Preexisting => Ok(()),
        }
    }
}

struct ExistingWalletUpgradeRollback<'a> {
    database: &'a Database,
    keychain: &'a Keychain,
    id: &'a WalletId,
    secret: UpgradeSecretState,
    previous_selected_wallet: Option<WalletId>,
}

impl ExistingWalletUpgradeRollback<'_> {
    fn after_selection_failure(&self, cause: &database::Error) -> Result<(), Error> {
        if self.secret.rollback(self.keychain, self.id).is_ok() {
            return Ok(());
        }

        Err(ImportWalletError::UpgradeRollback(format!(
            "database update: {cause}; secret rollback: {}",
            KeychainError::Delete
        )))
    }

    fn after_metadata_failure(&self, cause: &database::Error) -> Result<(), Error> {
        let selection_rollback = match &self.previous_selected_wallet {
            Some(previous_id) => self.database.global_config.select_wallet(previous_id.clone()),
            None => self.database.global_config.clear_selected_wallet(),
        };

        let secret_rollback = self.secret.rollback(self.keychain, self.id);

        if selection_rollback.is_ok() && secret_rollback.is_ok() {
            return Ok(());
        }

        let mut message = format!("metadata update: {cause}");

        if let Err(error) = selection_rollback {
            message.push_str(&format!("; selection rollback: {error}"));
        }

        if let Err(error) = secret_rollback {
            message.push_str(&format!("; secret rollback: {error}"));
        }

        Err(ImportWalletError::UpgradeRollback(message))
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

        let network = Database::global().global_config.selected_network();
        let mode = Database::global().global_config.wallet_mode();

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

    let existing_wallet =
        existing_wallet_for_secret(&database, keychain, &secret, network, mode, fingerprint)?;

    match existing_wallet {
        Some(metadata) => {
            upgrade_existing_wallet(secret, network, fingerprint, metadata, &database, keychain)
        }
        None => create_new_wallet(secret, network, mode, fingerprint, default_name, &database),
    }
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
        .map_err(|error| ImportWalletError::WalletImportError(error.to_string()))?;
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

fn upgrade_existing_wallet(
    secret: WalletSecret,
    network: Network,
    fingerprint: Fingerprint,
    mut metadata: WalletMetadata,
    database: &Database,
    keychain: &Keychain,
) -> Result<WalletMetadata, Error> {
    let id = metadata.id.clone();
    let _persistence = crate::wallet_lifecycle::WalletLifecycleCoordinator::global()
        .begin_persistence_operation(id.clone())
        .map_err(|error| ImportWalletError::WalletImportError(error.to_string()))?;
    let existing_secret = match keychain.get_wallet_secret(&id) {
        Ok(secret) => secret,
        Err(error) => {
            warn!("failed to read wallet secret for existing wallet {id}: {error}");

            None
        }
    };

    // hot wallets with private key already in keychain, don't do anything else
    if metadata.wallet_type == WalletType::Hot && existing_secret.is_some() {
        warn!("attempted to import a secret for existing hot wallet {id}, showing duplicate alert");

        return Err(ImportWalletError::WalletAlreadyExists(id));
    }

    info!("adding private key material to existing wallet {id}");

    let previous_selected_wallet = database.global_config.selected_wallet();
    let secret = match existing_secret {
        Some(existing_secret) => {
            let existing_fingerprint: Fingerprint =
                existing_secret.xpub(network).fingerprint().into();
            if existing_fingerprint != fingerprint {
                return Err(ImportWalletError::WalletIdentity(format!(
                    "stored secret for wallet {id} does not match the incoming fingerprint"
                )));
            }

            info!("keeping existing private key material for wallet {id}");

            UpgradeSecretState::Preexisting
        }
        None => {
            keychain.save_wallet_secret(&id, secret).map_err(|error| match error {
                KeychainError::WalletSecretExists => {
                    warn!(
                        "wallet {id} has a stored secret that is present but unreadable, showing duplicate alert"
                    );
                    ImportWalletError::WalletAlreadyExists(id.clone())
                }
                error => error.into(),
            })?;

            UpgradeSecretState::Created
        }
    };

    let rollback = ExistingWalletUpgradeRollback {
        database,
        keychain,
        id: &id,
        secret,
        previous_selected_wallet,
    };

    if let Err(error) = database.global_config.select_wallet(id.clone()) {
        rollback.after_selection_failure(&error)?;

        return Err(error.into());
    }

    // private key material means this wallet can now sign locally
    metadata.wallet_type = WalletType::Hot;
    metadata.hardware_metadata = None;
    metadata.verified = true;

    // commit metadata last so a database failure leaves the wallet non-hot and
    // requires rolling back only the selection and newly created secret
    metadata = match database.wallets.update_wallet_metadata(metadata) {
        Ok(metadata) => metadata,
        Err(error) => {
            rollback.after_metadata_failure(&error)?;

            return Err(error.into());
        }
    };

    Updater::send_update(Update::ClearCachedWalletManager(id));
    CLOUD_BACKUP_MANAGER.handle_wallet_backup_change_and_reverify(metadata.id.clone());

    Ok(metadata)
}

fn existing_wallet_for_secret(
    database: &Database,
    keychain: &Keychain,
    secret: &WalletSecret,
    network: Network,
    mode: WalletMode,
    fingerprint: Fingerprint,
) -> Result<Option<WalletMetadata>, Error> {
    let same_fingerprint_wallets = database
        .wallets
        .get_all(network, mode)?
        .into_iter()
        .filter(|metadata| metadata.matches_fingerprint(fingerprint))
        .collect::<Vec<_>>();
    if same_fingerprint_wallets.is_empty() {
        return Ok(None);
    }

    let mut exact_matches = Vec::new();
    let mut degraded_matches = Vec::new();

    for metadata in same_fingerprint_wallets {
        let incoming_descriptors = secret.clone().into_descriptors(network, metadata.address_type);
        let incoming_identity = PublicWalletIdentity::from_descriptors(&incoming_descriptors);
        let Some(existing_identity) =
            PublicWalletIdentity::from_existing_wallet(&metadata, keychain)?
        else {
            degraded_matches.push((metadata, incoming_identity.redacted_hash()));
            continue;
        };

        if existing_identity != incoming_identity {
            continue;
        }

        exact_matches.push(metadata);
    }

    if exact_matches.len() > 1 {
        return Err(ImportWalletError::WalletIdentityCollision);
    }

    if let Some(exact_match) = exact_matches.pop() {
        return Ok(Some(exact_match));
    }

    if degraded_matches.len() > 1 {
        return Err(ImportWalletError::WalletIdentityCollision);
    }

    let Some((degraded_match, incoming_identity_hash)) = degraded_matches.pop() else {
        return Ok(None);
    };

    let wallet_id = &degraded_match.id;
    warn!(
        "same-fingerprint wallet missing public identity wallet_id={wallet_id} incoming_identity_hash={incoming_identity_hash}, falling back to fingerprint match"
    );

    Ok(Some(degraded_match))
}

#[cfg(test)]
mod tests {
    use std::{
        str::FromStr as _,
        sync::{Arc, Once},
    };

    use super::*;
    use crate::wallet::WalletAddressType;
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
    fn failed_existing_wallet_upgrade_only_deletes_a_created_secret() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_globals();

        let database = Database::global();
        let keychain = Keychain::global();
        let original_selected_wallet = database.global_config.selected_wallet();
        let previous_selected_wallet = WalletId::new();
        let created_secret_wallet = WalletId::new();
        let preexisting_secret_wallet = WalletId::new();
        let preexisting_secret = xpriv_secret(36);
        let simulated_failure = database::Error::DatabaseAccess("simulated failure".to_string());
        database.global_config.select_wallet(previous_selected_wallet.clone()).unwrap();

        keychain.save_wallet_secret(&created_secret_wallet, xpriv_secret(37)).unwrap();
        database.global_config.select_wallet(created_secret_wallet.clone()).unwrap();

        ExistingWalletUpgradeRollback {
            database: database.as_ref(),
            keychain,
            id: &created_secret_wallet,
            secret: UpgradeSecretState::Created,
            previous_selected_wallet: Some(previous_selected_wallet.clone()),
        }
        .after_metadata_failure(&simulated_failure)
        .unwrap();

        assert_eq!(
            database.global_config.selected_wallet(),
            Some(previous_selected_wallet.clone())
        );
        assert!(keychain.get_wallet_secret(&created_secret_wallet).unwrap().is_none());

        keychain
            .save_wallet_secret(&preexisting_secret_wallet, preexisting_secret.clone())
            .unwrap();
        database.global_config.select_wallet(preexisting_secret_wallet.clone()).unwrap();

        ExistingWalletUpgradeRollback {
            database: database.as_ref(),
            keychain,
            id: &preexisting_secret_wallet,
            secret: UpgradeSecretState::Preexisting,
            previous_selected_wallet: Some(previous_selected_wallet.clone()),
        }
        .after_metadata_failure(&simulated_failure)
        .unwrap();

        assert_eq!(database.global_config.selected_wallet(), Some(previous_selected_wallet));
        assert_eq!(
            keychain.get_wallet_secret(&preexisting_secret_wallet).unwrap(),
            Some(preexisting_secret)
        );

        match original_selected_wallet {
            Some(id) => database.global_config.select_wallet(id).unwrap(),
            None => database.global_config.clear_selected_wallet().unwrap(),
        }
        assert!(keychain.delete_wallet_items(&created_secret_wallet));
        assert!(keychain.delete_wallet_items(&preexisting_secret_wallet));
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
