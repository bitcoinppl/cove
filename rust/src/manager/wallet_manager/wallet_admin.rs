use crate::{
    app::{
        AppError, FfiApp,
        reconcile::{Update, Updater},
    },
    database::Database,
    router::Route,
    wallet::metadata::{WalletMetadata, WalletType},
    wallet_lifecycle::{ShutdownAttemptId, ShutdownDeadlineTier},
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

        crate::app::delete_wallet_with_tier(wallet_id.clone(), ShutdownDeadlineTier::Initial, None)
            .await
            .map_err(wallet_deletion_error)?;

        self.finish_delete_wallet(wallet_id)
    }

    pub(crate) async fn retry_delete_wallet_internal(
        &self,
        attempt_id: ShutdownAttemptId,
    ) -> Result<(), Error> {
        let wallet_id = self.metadata.read().id.clone();
        crate::app::delete_wallet_with_tier(
            wallet_id.clone(),
            ShutdownDeadlineTier::Retry,
            Some(attempt_id),
        )
        .await
        .map_err(wallet_deletion_error)?;

        self.finish_delete_wallet(wallet_id)
    }

    fn finish_delete_wallet(
        &self,
        wallet_id: crate::wallet::metadata::WalletId,
    ) -> Result<(), Error> {
        let database = Database::global();

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

fn wallet_deletion_error(error: AppError) -> Error {
    match error {
        AppError::WalletLifecycle(failure) => Error::WalletLifecycle(failure),
        other => Error::DeleteWalletError(other.to_string()),
    }
}
