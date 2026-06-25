use std::sync::Arc;

use act_zero::send;
use tracing::{debug, warn};

use crate::{
    database::Database,
    transaction::{TxId, unsigned_transaction::UnsignedTransaction},
};
use cove_types::confirm::ConfirmDetails;

use super::{Error, Message, RustWalletManager};

impl RustWalletManager {
    pub(crate) fn save_unsigned_transaction_internal(
        &self,
        details: Arc<ConfirmDetails>,
    ) -> Result<(), Error> {
        let wallet_id = self.id.clone();
        let tx_id = details.psbt.tx_id();
        let db = Database::global();

        let confirm_details = Arc::unwrap_or_clone(details);

        let db = db.unsigned_transactions();

        if db.get_tx(&tx_id)?.is_some() {
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
        )?;

        self.reconciler.send(Message::UnsignedTransactionsChanged);

        Ok(())
    }
    pub(crate) fn get_unsigned_transactions_internal(
        &self,
    ) -> Result<Vec<Arc<UnsignedTransaction>>, Error> {
        let wallet_id = &self.id;

        let db = Database::global();
        let txns = db.unsigned_transactions().get_by_wallet_id(wallet_id)?;

        let txns = txns
            .into_iter()
            .map(|txn| Arc::new(txn.into()))
            .collect::<Vec<Arc<UnsignedTransaction>>>();

        Ok(txns)
    }

    pub(crate) fn delete_unsigned_transaction_internal(
        &self,
        tx_id: Arc<TxId>,
    ) -> Result<(), Error> {
        debug!("deleting unsigned transaction: {tx_id:?}");
        let db = Database::global();

        let txn = db.unsigned_transactions().delete_tx(tx_id.as_ref())?;
        send!(self.actor.cancel_txn(txn.confirm_details.psbt.0.unsigned_tx));

        self.reconciler.send(Message::UnsignedTransactionsChanged);

        Ok(())
    }
}
