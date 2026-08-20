use std::{fmt::Display, future::Future};

use crate::{
    app::{
        FfiApp,
        reconcile::{Update, Updater},
    },
    database::Database,
    router::Route,
    wallet::{
        deletion::WalletDeletion,
        metadata::{WalletMetadata, WalletType},
    },
};
use act_zero::{Actor, Addr, AddrLike as _, call};
use cove_util::result_ext::ResultExt as _;
use futures::FutureExt as _;
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
        WalletDeletion::new().delete(&wallet_id).map_err_str(Error::DeleteWalletError)?;

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
            shutdown_actor(
                discovery_scanner,
                call!(discovery_scanner.shutdown()),
                "wallet discovery scanner",
            )
            .await?;
        }

        shutdown_actor(&self.actor, call!(self.actor.shutdown()), "wallet actor").await?;

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

async fn shutdown_actor<T, F, E>(
    actor: &Addr<T>,
    shutdown: F,
    actor_name: &str,
) -> Result<(), Error>
where
    T: Actor,
    F: Future<Output = Result<(), E>>,
    E: Display,
{
    match shutdown.await {
        Ok(()) => Ok(()),
        Err(error) => {
            // a canceled actor call is idempotent only when the actor has already stopped
            tokio::task::yield_now().await;

            if actor.termination().now_or_never().is_some() {
                return Ok(());
            }

            Err(Error::UnknownError(format!("{actor_name} shutdown failed: {error}")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct FailingActor;

    #[async_trait::async_trait]
    impl Actor for FailingActor {
        async fn error(&mut self, _error: act_zero::ActorError) -> bool {
            false
        }
    }

    impl FailingActor {
        async fn shutdown(&mut self) -> act_zero::ActorResult<()> {
            Err("nested worker is still running".into())
        }
    }

    #[tokio::test]
    async fn shutdown_actor_accepts_a_detached_actor() {
        let actor = Addr::<FailingActor>::detached();

        shutdown_actor(&actor, call!(actor.shutdown()), "test actor")
            .await
            .expect("detached actors are already stopped");
    }

    #[tokio::test]
    async fn shutdown_actor_reports_an_active_shutdown_failure() {
        crate::test_support::ensure_tokio_runtime();

        let actor = cove_tokio::task::spawn_actor(FailingActor);

        let error = shutdown_actor(&actor, call!(actor.shutdown()), "test actor")
            .await
            .expect_err("an active actor failure must not be treated as already stopped");

        assert!(
            matches!(error, Error::UnknownError(message) if message.contains("test actor shutdown failed"))
        );
    }
}
