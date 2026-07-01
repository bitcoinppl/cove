use std::sync::Arc;

use act_zero::call;
use bitcoin::OutPoint;
use cove_util::result_ext::ResultExt as _;

use crate::transaction::TxId;

use super::{Error, RustWalletManager, TransactionLockState};

#[uniffi::export(async_runtime = "tokio")]
impl RustWalletManager {
    #[uniffi::method]
    pub async fn transaction_lock_state(
        &self,
        tx_id: Arc<TxId>,
    ) -> Result<TransactionLockState, Error> {
        let tx_id = Arc::unwrap_or_clone(tx_id);
        let state = call!(self.actor.transaction_lock_state(tx_id))
            .await
            .map_err(|_| Error::ActorNotFound)??;

        Ok(state)
    }

    #[uniffi::method]
    pub async fn toggle_transaction_lock_state(
        &self,
        tx_id: Arc<TxId>,
    ) -> Result<TransactionLockState, Error> {
        let tx_id = Arc::unwrap_or_clone(tx_id);
        let state = call!(self.actor.transaction_lock_state(tx_id))
            .await
            .map_err(|_| Error::ActorNotFound)??;
        let outpoints = call!(self.actor.current_wallet_unspent_outpoints_for_txn(tx_id))
            .await
            .map_err(|_| Error::ActorNotFound)?;
        let Some((outpoints, spendable)) = transaction_lock_toggle_update(state, outpoints) else {
            return Ok(TransactionLockState::None);
        };

        self.label_manager
            .set_output_spendability_for_outpoints(outpoints, spendable)
            .map_err_str(Error::OutputLabelsError)?;

        let state = call!(self.actor.transaction_lock_state(tx_id))
            .await
            .map_err(|_| Error::ActorNotFound)??;

        Ok(state)
    }

    #[uniffi::method]
    pub async fn unlock_transaction_outputs(
        &self,
        tx_id: Arc<TxId>,
    ) -> Result<TransactionLockState, Error> {
        let tx_id = Arc::unwrap_or_clone(tx_id);
        let outpoints = call!(self.actor.current_wallet_unspent_outpoints_for_txn(tx_id))
            .await
            .map_err(|_| Error::ActorNotFound)?;
        let Some((outpoints, spendable)) = transaction_unlock_update(outpoints) else {
            return Ok(TransactionLockState::None);
        };

        self.label_manager
            .set_output_spendability_for_outpoints(outpoints, spendable)
            .map_err_str(Error::OutputLabelsError)?;

        let state = call!(self.actor.transaction_lock_state(tx_id))
            .await
            .map_err(|_| Error::ActorNotFound)??;

        Ok(state)
    }
}

fn spendability_for_transaction_lock_toggle(state: TransactionLockState) -> Option<bool> {
    match state {
        TransactionLockState::None => None,
        TransactionLockState::Locked => Some(true),
        TransactionLockState::Unlocked | TransactionLockState::Mixed => Some(false),
    }
}

fn transaction_lock_toggle_update(
    state: TransactionLockState,
    outpoints: Vec<OutPoint>,
) -> Option<(Vec<OutPoint>, bool)> {
    let spendable = spendability_for_transaction_lock_toggle(state)?;
    if outpoints.is_empty() {
        return None;
    }

    Some((outpoints, spendable))
}

fn transaction_unlock_update(outpoints: Vec<OutPoint>) -> Option<(Vec<OutPoint>, bool)> {
    if outpoints.is_empty() {
        return None;
    }

    Some((outpoints, true))
}

#[cfg(test)]
mod tests {
    use bitcoin::{OutPoint, Txid, hashes::Hash as _};

    use super::{
        TransactionLockState, spendability_for_transaction_lock_toggle,
        transaction_lock_toggle_update, transaction_unlock_update,
    };

    fn outpoint(vout: u32) -> OutPoint {
        OutPoint { txid: Txid::from_byte_array([1; 32]), vout }
    }

    #[test]
    fn transaction_lock_toggle_decides_target_spendability_from_state() {
        assert_eq!(spendability_for_transaction_lock_toggle(TransactionLockState::None), None);
        assert_eq!(
            spendability_for_transaction_lock_toggle(TransactionLockState::Unlocked),
            Some(false)
        );
        assert_eq!(
            spendability_for_transaction_lock_toggle(TransactionLockState::Mixed),
            Some(false)
        );
        assert_eq!(
            spendability_for_transaction_lock_toggle(TransactionLockState::Locked),
            Some(true)
        );
    }

    #[test]
    fn transaction_lock_toggle_uses_current_outpoints_for_bulk_label_update() {
        let outpoints = vec![outpoint(0), outpoint(2)];

        assert_eq!(
            transaction_lock_toggle_update(TransactionLockState::Unlocked, outpoints.clone()),
            Some((outpoints.clone(), false))
        );
        assert_eq!(
            transaction_lock_toggle_update(TransactionLockState::Mixed, outpoints.clone()),
            Some((outpoints.clone(), false))
        );
        assert_eq!(
            transaction_lock_toggle_update(TransactionLockState::Locked, outpoints.clone()),
            Some((outpoints, true))
        );
    }

    #[test]
    fn transaction_lock_toggle_noops_without_current_outpoints() {
        assert_eq!(transaction_lock_toggle_update(TransactionLockState::None, vec![]), None);
        assert_eq!(transaction_lock_toggle_update(TransactionLockState::Unlocked, vec![]), None);
        assert_eq!(transaction_lock_toggle_update(TransactionLockState::Locked, vec![]), None);
    }

    #[test]
    fn transaction_unlock_update_marks_current_outpoints_spendable() {
        let outpoints = vec![outpoint(0), outpoint(2)];

        assert_eq!(transaction_unlock_update(outpoints.clone()), Some((outpoints, true)));
        assert_eq!(transaction_unlock_update(vec![]), None);
    }
}
