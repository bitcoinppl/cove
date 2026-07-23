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
}

pub type Error = ImportWalletError;

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

    // check if the wallet already exists using the fingerprint
    let existing_wallet = Database::global()
        .wallets
        .get_all(network, mode)?
        .into_iter()
        .find(|wallet_metadata| wallet_metadata.matches_fingerprint(fingerprint));

    // new wallet, create it and return
    if existing_wallet.is_none() {
        // get current number of wallets and add one;
        let number_of_wallets = Database::global().wallets.len(network, mode)?;

        let name = default_name.resolve(fingerprint, number_of_wallets);
        let mut wallet_metadata = match &secret {
            WalletSecret::Mnemonic(_) => {
                WalletMetadata::new_imported_from_mnemonic(name, network, fingerprint)
            }
            WalletSecret::Xpriv(_) => {
                WalletMetadata::new_imported_from_xpriv(name, network, fingerprint)
            }
        };
        wallet_metadata.wallet_mode = mode;

        match secret {
            WalletSecret::Mnemonic(mnemonic) => {
                Wallet::try_new_persisted_and_selected(wallet_metadata.clone(), mnemonic, None)
            }
            WalletSecret::Xpriv(xpriv) => {
                Wallet::try_new_persisted_xpriv_and_selected(wallet_metadata.clone(), xpriv)
            }
        }
        .map_err_str(ImportWalletError::WalletImportError)?;
        CLOUD_BACKUP_MANAGER.handle_wallet_set_change();

        return Ok(wallet_metadata);
    }

    // existing wallet
    let mut metadata = existing_wallet.expect("wallet exists, just checked above");
    let id = metadata.id.clone();
    let keychain = Keychain::global();

    // hot wallets with private key already in keychain, don't do anything else
    if metadata.wallet_type == WalletType::Hot && keychain.get_wallet_secret(&id)?.is_some() {
        warn!("attempted to import a secret for existing hot wallet {id}, showing duplicate alert");

        return Err(ImportWalletError::WalletAlreadyExists(id));
    }

    info!("adding private key material to existing wallet {id}");

    // save the private key material for an existing wallet.
    keychain.save_wallet_secret(&id, secret.clone())?;

    // save xpub/descriptors in keychain too
    let xpub = secret.xpub(network);
    keychain.save_wallet_xpub(&id, xpub)?;

    // save public descriptors in keychain too
    let descriptors = secret.into_descriptors(network, metadata.address_type);
    keychain.save_public_descriptor(
        &id,
        descriptors.external.extended_descriptor,
        descriptors.internal.extended_descriptor,
    )?;

    // imported mnemonic means this wallet can now sign locally.
    metadata.wallet_type = WalletType::Hot;
    metadata.hardware_metadata = None;
    metadata.verified = true;

    metadata = Database::global().wallets.update_wallet_metadata(metadata)?;
    Database::global().global_config.select_wallet(id.clone())?;
    Updater::send_update(Update::ClearCachedWalletManager(id));
    CLOUD_BACKUP_MANAGER.handle_wallet_backup_change_and_reverify(metadata.id.clone());

    Ok(metadata)
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        str::FromStr as _,
        sync::{Mutex, Once},
    };

    use super::*;
    use crate::keychain::KeychainAccess;
    use bdk_wallet::bitcoin::bip32::Xpriv;
    use cove_device::keychain::WalletXprv;

    #[derive(Debug, Default)]
    struct TestKeychain(Mutex<HashMap<String, String>>);

    impl KeychainAccess for TestKeychain {
        fn save(&self, key: String, value: String) -> Result<(), KeychainError> {
            self.0.lock().unwrap().insert(key, value);

            Ok(())
        }

        fn get(&self, key: String) -> Option<String> {
            self.0.lock().unwrap().get(&key).cloned()
        }

        fn delete(&self, key: String) -> bool {
            self.0.lock().unwrap().remove(&key).is_some()
        }
    }

    fn init_globals() {
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            crate::test_support::ensure_tokio_runtime();
            crate::database::test_support::init_test_database();
            let _ = Keychain::new(Box::<TestKeychain>::default());
        });
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
}
