use cove_device::keychain::WalletSecret;
use cove_util::result_ext::ResultExt as _;
use tracing::{info, warn};

use crate::{
    app::reconcile::{Update, Updater},
    database::{self, Database},
    keychain::{Keychain, KeychainError},
    network::Network,
    wallet::{
        fingerprint::Fingerprint,
        metadata::{WalletId, WalletMetadata, WalletMode, WalletType},
    },
    wallet_identity::{PublicWalletIdentity, PublicWalletIdentityError},
    wallet_secret::WalletSecretExt as _,
};

/// Why a secret could not be added to an existing wallet
#[derive(Debug, thiserror::Error)]
pub(crate) enum SecretUpgradeError {
    #[error("wallet already holds a private key")]
    AlreadyHoldsSecret(WalletId),

    #[error("multiple existing wallets could match the incoming secret")]
    IdentityCollision,

    #[error("failed to verify existing wallet public identity: {0}")]
    Identity(String),

    #[error("failed to save wallet to keychain: {0}")]
    Keychain(#[from] KeychainError),

    #[error("failed to save wallet: {0}")]
    Database(#[from] database::Error),

    #[error("wallet is not available for changes: {0}")]
    Lifecycle(String),

    #[error("failed to roll back an incomplete existing-wallet upgrade: {0}")]
    Rollback(String),
}

impl From<PublicWalletIdentityError> for SecretUpgradeError {
    fn from(error: PublicWalletIdentityError) -> Self {
        Self::Identity(error.to_string())
    }
}

/// Where an upgrade's secret came from, which decides how the upgraded wallet is presented
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SecretUpgradeSource {
    /// The user entered or received the secret and moves to that wallet now
    Import,

    /// A cloud backup supplied the secret, so the selected wallet stays as it is and the backup's
    /// own record decides whether the words count as verified
    CloudBackup { verified: bool },
}

/// A local public wallet that gains the private key for the identity it already watches
///
/// Every path that hands a secret to an existing wallet goes through this owner, so the wallet
/// match, the writes, and their rollback stay the same for imports and cloud restores
#[derive(Debug)]
pub(crate) struct ExistingWalletSecretUpgrade {
    secret: WalletSecret,
    network: Network,
    fingerprint: Fingerprint,
    target: WalletMetadata,
}

impl ExistingWalletSecretUpgrade {
    /// Finds the local wallet with this secret's public identity, or `None` when the secret
    /// belongs to no local wallet and would be a new one
    pub(crate) fn find(
        database: &Database,
        keychain: &Keychain,
        secret: &WalletSecret,
        network: Network,
        mode: WalletMode,
    ) -> Result<Option<Self>, SecretUpgradeError> {
        let fingerprint: Fingerprint = secret.xpub(network).fingerprint().into();
        let target =
            existing_wallet_for_secret(database, keychain, secret, network, mode, fingerprint)?;

        Ok(target.map(|target| Self { secret: secret.clone(), network, fingerprint, target }))
    }

    /// Saves the secret for the matched wallet and marks it hot, rolling back on failure
    ///
    /// The caller tells Cloud Backup about the changed wallet, since an import and a restore that
    /// is still running need different follow-up work
    pub(crate) fn apply(
        self,
        database: &Database,
        keychain: &Keychain,
        source: SecretUpgradeSource,
    ) -> Result<WalletMetadata, SecretUpgradeError> {
        let Self { secret, network, fingerprint, target: mut metadata } = self;
        let id = metadata.id.clone();
        let _persistence = crate::wallet_lifecycle::WalletLifecycleCoordinator::global()
            .begin_persistence_operation(id.clone())
            .map_err_str(SecretUpgradeError::Lifecycle)?;
        let existing_secret = match keychain.get_wallet_secret(&id) {
            Ok(secret) => secret,
            Err(error) => {
                warn!("failed to read wallet secret for existing wallet {id}: {error}");

                None
            }
        };

        // hot wallets with private key already in keychain, don't do anything else
        if metadata.wallet_type == WalletType::Hot && existing_secret.is_some() {
            warn!(
                "attempted to add a secret to existing hot wallet {id}, treating it as a duplicate"
            );

            return Err(SecretUpgradeError::AlreadyHoldsSecret(id));
        }

        info!("adding private key material to existing wallet {id}");

        let secret = match existing_secret {
            Some(existing_secret) => {
                let existing_fingerprint: Fingerprint =
                    existing_secret.xpub(network).fingerprint().into();
                if existing_fingerprint != fingerprint {
                    return Err(SecretUpgradeError::Identity(format!(
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
                            "wallet {id} has a stored secret that is present but unreadable, treating it as a duplicate"
                        );
                        SecretUpgradeError::AlreadyHoldsSecret(id.clone())
                    }
                    error => error.into(),
                })?;

                UpgradeSecretState::Created
            }
        };

        let mut rollback = ExistingWalletUpgradeRollback {
            database,
            keychain,
            id: &id,
            secret,
            selection: UpgradeSelection::Unchanged,
        };

        // an import moves the user to the wallet they just entered, a restore leaves them where
        // they are
        if source == SecretUpgradeSource::Import {
            let previous = database.global_config.selected_wallet();
            if let Err(error) = database.global_config.select_wallet(id.clone()) {
                rollback.after_selection_failure(&error)?;

                return Err(error.into());
            }

            rollback.selection = UpgradeSelection::Selected { previous };
        }

        // private key material means this wallet can now sign locally
        metadata.wallet_type = WalletType::Hot;
        metadata.hardware_metadata = None;
        metadata.verified = match source {
            SecretUpgradeSource::Import => true,
            SecretUpgradeSource::CloudBackup { verified } => metadata.verified || verified,
        };

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

        Ok(metadata)
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

/// Whether an upgrade changed the selected wallet, and what to put back if it did
enum UpgradeSelection {
    Unchanged,
    Selected { previous: Option<WalletId> },
}

struct ExistingWalletUpgradeRollback<'a> {
    database: &'a Database,
    keychain: &'a Keychain,
    id: &'a WalletId,
    secret: UpgradeSecretState,
    selection: UpgradeSelection,
}

impl ExistingWalletUpgradeRollback<'_> {
    fn after_selection_failure(&self, cause: &database::Error) -> Result<(), SecretUpgradeError> {
        if self.secret.rollback(self.keychain, self.id).is_ok() {
            return Ok(());
        }

        Err(SecretUpgradeError::Rollback(format!(
            "database update: {cause}; secret rollback: {}",
            KeychainError::Delete
        )))
    }

    fn after_metadata_failure(&self, cause: &database::Error) -> Result<(), SecretUpgradeError> {
        let selection_rollback = match &self.selection {
            UpgradeSelection::Unchanged => Ok(()),
            UpgradeSelection::Selected { previous: Some(previous_id) } => {
                self.database.global_config.select_wallet(previous_id.clone())
            }
            UpgradeSelection::Selected { previous: None } => {
                self.database.global_config.clear_selected_wallet()
            }
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

        Err(SecretUpgradeError::Rollback(message))
    }
}

fn existing_wallet_for_secret(
    database: &Database,
    keychain: &Keychain,
    secret: &WalletSecret,
    network: Network,
    mode: WalletMode,
    fingerprint: Fingerprint,
) -> Result<Option<WalletMetadata>, SecretUpgradeError> {
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
        return Err(SecretUpgradeError::IdentityCollision);
    }

    if let Some(exact_match) = exact_matches.pop() {
        return Ok(Some(exact_match));
    }

    if degraded_matches.len() > 1 {
        return Err(SecretUpgradeError::IdentityCollision);
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
    use bdk_wallet::bitcoin::bip32::Xpriv;
    use cove_device::keychain::WalletXprv;

    use super::*;

    fn xpriv_secret(seed_byte: u8) -> WalletSecret {
        let xpriv =
            Xpriv::new_master(bdk_wallet::bitcoin::Network::Bitcoin, &[seed_byte; 32]).unwrap();

        WalletSecret::Xpriv(WalletXprv::try_from(xpriv).unwrap())
    }

    #[test]
    fn failed_existing_wallet_upgrade_only_deletes_a_created_secret() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        crate::test_support::ensure_tokio_runtime();
        crate::database::test_support::init_test_database();
        crate::test_support::init_test_keychain();

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
            selection: UpgradeSelection::Selected {
                previous: Some(previous_selected_wallet.clone()),
            },
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
            selection: UpgradeSelection::Selected {
                previous: Some(previous_selected_wallet.clone()),
            },
        }
        .after_metadata_failure(&simulated_failure)
        .unwrap();

        assert_eq!(database.global_config.selected_wallet(), Some(previous_selected_wallet));
        assert_eq!(
            keychain.get_wallet_secret(&preexisting_secret_wallet).unwrap(),
            Some(preexisting_secret)
        );

        // a restore never selected the wallet, so its rollback leaves the selection alone
        let restored_secret_wallet = WalletId::new();
        keychain.save_wallet_secret(&restored_secret_wallet, xpriv_secret(39)).unwrap();
        database.global_config.select_wallet(preexisting_secret_wallet.clone()).unwrap();

        ExistingWalletUpgradeRollback {
            database: database.as_ref(),
            keychain,
            id: &restored_secret_wallet,
            secret: UpgradeSecretState::Created,
            selection: UpgradeSelection::Unchanged,
        }
        .after_metadata_failure(&simulated_failure)
        .unwrap();

        assert_eq!(
            database.global_config.selected_wallet(),
            Some(preexisting_secret_wallet.clone())
        );
        assert!(keychain.get_wallet_secret(&restored_secret_wallet).unwrap().is_none());

        match original_selected_wallet {
            Some(id) => database.global_config.select_wallet(id).unwrap(),
            None => database.global_config.clear_selected_wallet().unwrap(),
        }
        assert!(keychain.delete_wallet_items(&created_secret_wallet));
        assert!(keychain.delete_wallet_items(&preexisting_secret_wallet));
    }
}
