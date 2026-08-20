use std::path::PathBuf;

use tracing::error;

use crate::{database::Database, keychain::Keychain};

use super::{addressing, metadata::WalletId, remove_wallet_specific_artifacts};

#[derive(Debug, thiserror::Error)]
pub(crate) enum WalletDeletionError {
    #[error("unable to remove address switch recovery data: {0}")]
    AddressSwitchRecovery(#[source] std::io::Error),

    #[error("unable to remove wallet metadata: {0}")]
    Metadata(#[from] crate::database::Error),
}

/// Owns the ordering between address-switch recovery and wallet deletion
pub(crate) struct WalletDeletion(PathBuf);

/// Wallet-data wipe whose address-switch recovery state is already invalidated
pub(crate) struct PreparedWalletDataWipe(());

impl WalletDeletion {
    /// Create a deletion coordinator for the production journal directory
    pub(crate) fn new() -> Self {
        Self(addressing::switch_directory_root())
    }

    /// Delete one registered wallet after its recovery state is durably invalidated
    pub(crate) fn delete(&self, wallet_id: &WalletId) -> Result<(), WalletDeletionError> {
        addressing::remove_address_switch_journals_in(&self.0, wallet_id)
            .map_err(WalletDeletionError::AddressSwitchRecovery)?;

        Database::global().wallets.delete(wallet_id)?;
        Keychain::global().delete_wallet_items(wallet_id);

        if let Err(error) = remove_wallet_specific_artifacts(wallet_id) {
            error!(%error, "Unable to delete wallet persisted data");
        }

        Ok(())
    }

    /// Invalidate all recovery state before the main database is reset
    pub(crate) fn prepare_full_wipe(self) -> Result<PreparedWalletDataWipe, WalletDeletionError> {
        addressing::remove_all_address_switch_journals_in(&self.0)
            .map_err(WalletDeletionError::AddressSwitchRecovery)?;

        Ok(PreparedWalletDataWipe(()))
    }
}

impl PreparedWalletDataWipe {
    /// Remove one wallet's data after recovery has been invalidated for the full wipe
    pub(crate) fn delete(&self, wallet_id: &WalletId) {
        if let Err(error) = Database::global().wallets.delete(wallet_id) {
            error!(%error, "Unable to delete wallet metadata during data wipe");
        }

        Keychain::global().delete_wallet_items(wallet_id);

        if let Err(error) = remove_wallet_specific_artifacts(wallet_id) {
            error!(%error, "Unable to delete wallet persisted data during data wipe");
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::wallet::metadata::WalletMetadata;

    #[tokio::test(flavor = "current_thread")]
    async fn journal_directory_read_failure_preserves_wallet_metadata() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;

        crate::test_support::ensure_tokio_runtime();
        crate::test_support::init_test_keychain();
        crate::database::test_support::init_test_database();
        let mut metadata = WalletMetadata::preview_new();
        metadata.id = WalletId::preview_new_random();
        let database = Database::global();
        database
            .wallets
            .save_new_wallet_metadata(metadata.clone())
            .expect("wallet metadata is saved");

        let directory = TempDir::new().expect("temporary directory is created");
        let journal_root = directory.path().join("not-a-directory");
        std::fs::write(&journal_root, b"not a directory").expect("invalid journal root is created");
        let deletion = WalletDeletion(journal_root);

        let result = deletion.delete(&metadata.id);

        assert!(
            matches!(result, Err(WalletDeletionError::AddressSwitchRecovery(_))),
            "journal cleanup failure is returned"
        );
        assert_eq!(
            database
                .wallets
                .get(&metadata.id, metadata.network, metadata.wallet_mode)
                .expect("wallet metadata is read"),
            Some(metadata.clone()),
            "wallet metadata remains recoverable after failed deletion"
        );

        database.wallets.delete(&metadata.id).expect("test wallet metadata is deleted");
    }
}
