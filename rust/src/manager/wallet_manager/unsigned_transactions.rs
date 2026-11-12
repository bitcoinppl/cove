use std::sync::Arc;

use act_zero::send;
use tracing::{debug, error, warn};

use crate::{
    database::{
        Database, error::DatabaseError, unsigned_transactions::UnsignedTransactionsTableError,
    },
    transaction::{TxId, unsigned_transaction::UnsignedTransaction},
};
use cove_types::confirm::ConfirmDetails;

use super::{Error, Message, RustWalletManager, unsigned_transactions_for_wallet};

impl RustWalletManager {
    pub(crate) fn save_unsigned_transaction_internal(
        &self,
        details: Arc<ConfirmDetails>,
    ) -> Result<(), Error> {
        if self.unsigned_transactions.is_in_memory() {
            return Ok(());
        }

        let wallet_id = self.id.clone();
        let tx_id = details.psbt.tx_id();
        let db = Database::global();

        let confirm_details = Arc::unwrap_or_clone(details);

        let db = db.unsigned_transactions();

        let existing = db.get_tx(&tx_id).map_err(|error| {
            error!("failed to check for an existing unsigned transaction: {error}");
            Error::SaveUnsignedTransactionError(error)
        })?;

        if existing.is_some() {
            warn!("tx {} already exists", tx_id.0.to_raw_hash().to_string());
            return Ok(());
        }

        // save the tx to the database
        db.save_tx(
            tx_id,
            UnsignedTransaction {
                wallet_id,
                tx_id,
                confirm_details,
                created_at: jiff::Timestamp::now().as_second() as u64,
            }
            .into(),
        )
        .map_err(|error| {
            error!("failed to save unsigned transaction: {error}");
            Error::SaveUnsignedTransactionError(error)
        })?;

        self.reconciler.send(Message::UnsignedTransactionsChanged);

        Ok(())
    }
    pub(crate) fn get_unsigned_transactions_internal(
        &self,
    ) -> Result<Vec<Arc<UnsignedTransaction>>, Error> {
        if self.unsigned_transactions.is_in_memory() {
            return Ok(Vec::new());
        }

        unsigned_transactions_for_wallet(&self.id)
    }

    pub(crate) fn delete_unsigned_transaction_internal(
        &self,
        tx_id: Arc<TxId>,
    ) -> Result<(), Error> {
        if self.unsigned_transactions.is_in_memory() {
            return Ok(());
        }

        debug!("deleting unsigned transaction: {tx_id:?}");
        let db = Database::global();

        let txn = db.unsigned_transactions().delete_tx(tx_id.as_ref()).map_err(|error| {
            // a draft already deleted by another screen is a user race, not a storage fault
            match &error {
                DatabaseError::UnsignedTransactions(
                    UnsignedTransactionsTableError::NoRecordFound,
                ) => {
                    warn!("unsigned transaction to delete was already gone");
                }
                _ => error!("failed to delete unsigned transaction: {error}"),
            }

            Error::DeleteUnsignedTransactionError(error)
        })?;

        send!(self.actor.cancel_txn(txn.confirm_details.psbt.0.unsigned_tx));

        self.reconciler.send(Message::UnsignedTransactionsChanged);

        Ok(())
    }
}
