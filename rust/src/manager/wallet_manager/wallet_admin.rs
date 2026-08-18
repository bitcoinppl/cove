use crate::{
    app::{
        FfiApp,
        reconcile::{Update, Updater},
    },
    database::Database,
    keychain::Keychain,
    router::Route,
    wallet::metadata::{WalletMetadata, WalletType},
};
use act_zero::call;
use cove_util::result_ext::ResultExt as _;
use tap::TapFallible as _;
use tracing::error;

use super::{Error, RustWalletManager};

impl RustWalletManager {
    pub(crate) async fn delete_wallet_internal(&self) -> Result<(), Error> {
        if !self.uses_persistent_storage() {
            return Err(Error::PreviewOperationUnavailable);
        }

        let wallet_id = self.metadata.read().id.clone();
        tracing::debug!("deleting wallet {wallet_id}");

        // deletion is a lifecycle boundary, so wait for workers before removing persisted data
        self.shutdown_actors_and_wait().await?;

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

    async fn shutdown_actors_and_wait(&self) -> Result<(), Error> {
        if let Some(discovery_scanner) = &self.discovery_scanner {
            call!(discovery_scanner.shutdown()).await.map_err(|_| Error::ActorNotFound)?;
        }

        call!(self.actor.shutdown()).await.map_err(|_| Error::ActorNotFound)?;

        Ok(())
    }

    pub(crate) async fn set_wallet_type_internal(
        &self,
        wallet_type: WalletType,
    ) -> Result<(), Error> {
        let result = call!(self.actor.set_wallet_type(wallet_type))
            .await
            .map_err(|_| Error::ActorNotFound)?;
        result.map_err_str(Error::SetWalletTypeError)?;

        Ok(())
    }
    pub(crate) async fn validate_metadata_internal(&self) -> Result<(), Error> {
        call!(self.actor.validate_metadata()).await.map_err(|_| Error::ActorNotFound)??;

        Ok(())
    }
    pub(crate) async fn mark_wallet_as_verified_internal(&self) -> Result<(), Error> {
        call!(self.actor.mark_wallet_as_verified()).await.map_err(|_| Error::ActorNotFound)??;

        Ok(())
    }
    pub(crate) fn wallet_metadata_internal(&self) -> WalletMetadata {
        self.metadata.read().clone()
    }
}
