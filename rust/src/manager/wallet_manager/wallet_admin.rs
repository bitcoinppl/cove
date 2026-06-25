use crate::{
    app::{
        FfiApp,
        reconcile::{Update, Updater},
    },
    database::Database,
    keychain::Keychain,
    manager::cloud_backup_manager::CLOUD_BACKUP_MANAGER,
    router::Route,
    wallet::{
        fingerprint::Fingerprint,
        metadata::{WalletMetadata, WalletType},
    },
};
use cove_util::result_ext::ResultExt as _;
use tap::TapFallible as _;
use tracing::error;

use super::{Error, Message, RustWalletManager};

impl RustWalletManager {
    pub(crate) fn delete_wallet_internal(&self) -> Result<(), Error> {
        let wallet_id = self.metadata.read().id.clone();
        tracing::debug!("deleting wallet {wallet_id}");

        let database = Database::global();
        let keychain = Keychain::global();

        // delete the wallet from the database
        database.wallets.delete(&wallet_id)?;

        // delete the secret key, xpub and public descriptor from the keychain
        keychain.delete_wallet_items(&wallet_id);

        // delete the wallet persisted bdk data
        if let Err(error) = crate::wallet::delete_wallet_specific_data(&wallet_id) {
            error!("Unable to delete wallet persisted bdk data and wallet data database: {error}");
        }

        Updater::send_update(Update::ClearCachedWalletManager(wallet_id.clone()));

        // unselect the wallet in the database
        match database.global_config.selected_wallet() {
            Some(selected_wallet_id) if selected_wallet_id == wallet_id => {
                let _ = database.global_config.clear_selected_wallet().tap_err(|error| {
                    error!("Unable to clear selected wallet: {error}");
                });
            }
            _ => (),
        }

        // check if other wallets exist and select the first one, or go to new wallet flow
        let remaining_wallets = database.wallets().all().unwrap_or_default();
        if let Some(next_wallet) = remaining_wallets.first() {
            let _ = FfiApp::global().select_wallet(next_wallet.id.clone(), None);
        } else {
            // no wallets remaining, go to new wallet flow
            FfiApp::global().load_and_reset_default_route(Route::NewWallet(Default::default()));
        }

        Ok(())
    }
    pub(crate) fn set_wallet_type_internal(&self, wallet_type: WalletType) -> Result<(), Error> {
        let before_metadata = self.metadata.read().clone();
        let mut metadata = before_metadata.clone();
        metadata.wallet_type = wallet_type;

        metadata = Database::global()
            .wallets
            .update_wallet_metadata(metadata.clone())
            .map_err_debug(Error::SetWalletTypeError)?;

        *self.metadata.write() = metadata.clone();
        self.reconciler.send(Message::WalletMetadataChanged(Box::new(metadata.clone())));

        CLOUD_BACKUP_MANAGER.handle_wallet_metadata_update(&before_metadata, &metadata);

        Ok(())
    }
    pub(crate) fn validate_metadata_internal(&self) {
        let before_metadata = self.metadata.read().clone();
        if !before_metadata.name.trim().is_empty() {
            return;
        }

        let name = before_metadata
            .master_fingerprint
            .as_deref()
            .map_or_else(|| "Unnamed Wallet".to_string(), Fingerprint::as_uppercase);
        let mut metadata = before_metadata.clone();
        metadata.name = name;

        let metadata = match Database::global().wallets.update_wallet_metadata(metadata.clone()) {
            Ok(metadata) => metadata,
            Err(error) => {
                error!("Unable to update wallet metadata: {error:?}");
                return;
            }
        };

        *self.metadata.write() = metadata.clone();
        self.reconciler.send(Message::WalletMetadataChanged(Box::new(metadata.clone())));
        CLOUD_BACKUP_MANAGER.handle_wallet_metadata_update(&before_metadata, &metadata);
    }
    pub(crate) fn mark_wallet_as_verified_internal(&self) -> Result<(), Error> {
        // clone metadata and release lock before I/O
        let metadata = {
            let mut wallet_metadata = self.metadata.write();
            wallet_metadata.verified = true;
            wallet_metadata.clone()
        };

        Database::global().wallets.mark_wallet_as_verified(&metadata.id)?;

        self.reconciler.send(Message::WalletMetadataChanged(Box::new(metadata.clone())));

        Ok(())
    }
    pub(crate) fn wallet_metadata_internal(&self) -> WalletMetadata {
        self.metadata.read().clone()
    }
}
