use crate::{database::Database, keychain::Keychain};

use super::metadata::WalletId;

#[derive(Debug, thiserror::Error)]
pub(crate) enum WalletDeletionError {
    #[error("unable to remove wallet keychain entries")]
    Keychain,

    #[error("unable to remove wallet data: {0}")]
    Artifacts(String),

    #[error("unable to remove wallet metadata: {0}")]
    Metadata(#[from] crate::database::Error),
}

/// Delete one registered wallet's secrets, artifacts, and metadata
///
/// Secrets go first and the canonical metadata row goes last: the row is the
/// durable record that a wallet exists, so a failure or crash at any earlier
/// step leaves the wallet listed and the user retries an idempotent deletion.
/// The reverse order would strand keychain secrets with no UI handle
pub(crate) fn delete_wallet(wallet_id: &WalletId) -> Result<(), WalletDeletionError> {
    if !Keychain::global().delete_wallet_items(wallet_id) {
        return Err(WalletDeletionError::Keychain);
    }

    super::delete_wallet_specific_data(wallet_id)
        .map_err(|error| WalletDeletionError::Artifacts(error.to_string()))?;

    Database::global().wallets.delete(wallet_id)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallet::metadata::WalletMetadata;

    #[tokio::test(flavor = "current_thread")]
    async fn deletion_is_idempotent_and_removes_the_metadata_row_last() {
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

        delete_wallet(&metadata.id).expect("wallet deletion succeeds");

        assert_eq!(
            database
                .wallets
                .get(&metadata.id, metadata.network, metadata.wallet_mode)
                .expect("wallet metadata is read"),
            None,
            "the metadata row is removed"
        );

        // a retry after partial cleanup must succeed on already-deleted state
        delete_wallet(&metadata.id).expect("wallet deletion is idempotent");
    }
}
