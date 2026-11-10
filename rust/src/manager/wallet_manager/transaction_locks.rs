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
        self.ensure_active()?;
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
        self.ensure_active()?;
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
        self.ensure_active()?;
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
    use std::{collections::HashSet, sync::Arc};

    use act_zero::call;
    use bitcoin::{OutPoint, Txid, hashes::Hash as _};

    use super::{
        TransactionLockState, spendability_for_transaction_lock_toggle,
        transaction_lock_toggle_update, transaction_unlock_update,
    };
    use crate::{
        manager::{
            coin_control_manager::CoinControlManagerAction,
            wallet_manager::{RustWalletManager, WalletManagerError},
        },
        test_support::{
            hardware_signing::external_recipient,
            utxo_locks::{LockTestWallet, OfflineChain},
        },
        transaction::{Amount, FeeRate, TxId},
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

    fn fee_rate() -> FeeRate {
        FeeRate::from_sat_per_vb(2.0)
    }

    fn psbt_inputs(psbt: &bitcoin::Psbt) -> HashSet<OutPoint> {
        psbt.unsigned_tx.input.iter().map(|input| input.previous_output).collect()
    }

    async fn lock_state(manager: &RustWalletManager, txid: Txid) -> TransactionLockState {
        manager.transaction_lock_state(Arc::new(TxId::from(txid))).await.expect("lock state reads")
    }

    async fn toggle(manager: &RustWalletManager, txid: Txid) -> TransactionLockState {
        manager
            .toggle_transaction_lock_state(Arc::new(TxId::from(txid)))
            .await
            .expect("lock toggles")
    }

    /// A transaction with four outputs to the wallet, the first and last of them since spent,
    /// plus a separate coin
    struct MixedTransaction {
        wallet: LockTestWallet,
        spent: [OutPoint; 2],
        unspent: [OutPoint; 2],
        separate: OutPoint,
    }

    impl MixedTransaction {
        fn txid(&self) -> Txid {
            self.unspent[0].txid
        }
    }

    fn mixed_transaction() -> MixedTransaction {
        let mut chain = OfflineChain::new();
        let outputs = chain.receive(&[10_000, 25_000, 15_000, 5_000]);
        chain.spend_to_external(&[outputs[0], outputs[3]]);
        let separate = chain.receive(&[39_000])[0];

        MixedTransaction {
            wallet: chain.open(),
            spent: [outputs[0], outputs[3]],
            unspent: [outputs[1], outputs[2]],
            separate,
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn locking_a_received_transaction_marks_it_locked_and_keeps_its_outputs_out_of_sends() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;
        let mut chain = OfflineChain::new();
        let received = chain.receive(&[30_000, 20_000]);
        let other = chain.receive(&[70_000])[0];
        let wallet = chain.open();
        let manager = &wallet.manager;
        let txid = received[0].txid;

        assert_eq!(lock_state(manager, txid).await, TransactionLockState::Unlocked);
        assert_eq!(toggle(manager, txid).await, TransactionLockState::Locked);

        // the transaction list row and the details screen both read this state
        assert_eq!(lock_state(manager, txid).await, TransactionLockState::Locked);
        assert!(received.iter().all(|outpoint| wallet.is_locked(*outpoint)));
        assert!(!wallet.is_locked(other));

        let unlocked_balance =
            manager.unlocked_spendable_balance().await.expect("unlocked balance reads");

        assert_eq!(unlocked_balance.as_sats(), 70_000);

        // a normal send funds itself from the other coin only
        let recipient = Arc::new(external_recipient());
        let details = manager
            .confirm_txn(Amount::from_sat(40_000), recipient.clone(), fee_rate())
            .await
            .expect("the unlocked coin funds the send");

        assert_eq!(psbt_inputs(&details.psbt.0), HashSet::from([other]));

        // a send only the locked outputs could complete fails instead of spending them
        let error = manager
            .confirm_txn(Amount::from_sat(90_000), recipient.clone(), fee_rate())
            .await
            .expect_err("locked outputs cannot top up a send");

        assert!(matches!(error, WalletManagerError::InsufficientFunds(_)), "{error:?}");

        // Max drains only the other coin
        let max =
            call!(manager.actor.build_ephemeral_drain_tx(recipient.as_ref().clone(), fee_rate()))
                .await
                .expect("wallet actor responds")
                .expect("max send builds");

        assert_eq!(psbt_inputs(&max), HashSet::from([other]));

        // coin control shows both outputs locked, Select All skips them, and naming them in a
        // manual send is rejected
        let coin_control =
            manager.new_coin_control_manager().await.expect("coin control manager opens");

        let locked_rows = coin_control
            .utxos()
            .into_iter()
            .filter(|utxo| received.contains(&bitcoin::OutPoint::from(utxo.outpoint.as_ref())))
            .collect::<Vec<_>>();

        assert_eq!(locked_rows.len(), received.len());
        assert!(locked_rows.iter().all(|utxo| !utxo.spendable));

        coin_control.clone().dispatch(CoinControlManagerAction::ToggleSelectAll);
        let selected = coin_control
            .selected_utxos()
            .iter()
            .map(|utxo| bitcoin::OutPoint::from(utxo.outpoint.as_ref()))
            .collect::<Vec<_>>();

        assert_eq!(selected, vec![other]);

        let error = manager
            .confirm_manual_txn(received.clone(), Amount::from_sat(45_000), recipient, fee_rate())
            .await
            .expect_err("locked outputs cannot be selected by hand");

        assert!(matches!(error, WalletManagerError::LockedOutputsSelected), "{error:?}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn toggling_a_mixed_transaction_locks_then_unlocks_every_unspent_output() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;
        let fixture = mixed_transaction();
        let wallet = &fixture.wallet;
        let manager = &wallet.manager;
        let [first, second] = fixture.unspent;

        // one output locked from coin control, and an unrelated lock elsewhere in the wallet
        let coin_control =
            manager.new_coin_control_manager().await.expect("coin control manager opens");

        coin_control
            .set_utxo_spendability(Arc::new(first.into()), false)
            .await
            .expect("coin control locks the output");

        wallet.lock(&[fixture.separate]);

        assert_eq!(lock_state(manager, fixture.txid()).await, TransactionLockState::Mixed);

        assert_eq!(toggle(manager, fixture.txid()).await, TransactionLockState::Locked);
        assert!(wallet.is_locked(first) && wallet.is_locked(second));

        assert_eq!(toggle(manager, fixture.txid()).await, TransactionLockState::Unlocked);
        assert!(!wallet.is_locked(first) && !wallet.is_locked(second));

        assert!(wallet.is_locked(fixture.separate), "toggling one transaction kept another lock");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn toggling_a_transaction_lock_leaves_its_spent_outputs_labels_and_locks_alone() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;
        let fixture = mixed_transaction();
        let wallet = &fixture.wallet;
        let manager = &wallet.manager;
        let [spent_spendable, spent_locked] = fixture.spent;
        let [first, second] = fixture.unspent;

        // one spent output recorded as spendable and one as locked, so a toggle that reached
        // spent outputs would flip one of them whichever way it went
        let labels = format!(
            "{}\n{}\n{}",
            format_args!(
                r#"{{"type":"output","ref":"{spent_spendable}","label":"spent spendable","spendable":true}}"#
            ),
            format_args!(
                r#"{{"type":"output","ref":"{spent_locked}","label":"spent locked","spendable":false}}"#
            ),
            format_args!(r#"{{"type":"output","ref":"{first}","label":"unspent label"}}"#),
        );

        manager.label_manager().import(&labels).expect("labels import");

        let spent_records = || fixture.spent.map(|outpoint| wallet.output_record(outpoint));
        let before = spent_records();
        assert!(before.iter().all(Option::is_some), "spent outputs have label records");

        wallet.lock(&[first]);
        assert_eq!(lock_state(manager, fixture.txid()).await, TransactionLockState::Mixed);

        assert_eq!(toggle(manager, fixture.txid()).await, TransactionLockState::Locked);
        assert_eq!(spent_records(), before, "locking changed a spent output's record");

        assert_eq!(toggle(manager, fixture.txid()).await, TransactionLockState::Unlocked);
        assert_eq!(spent_records(), before, "unlocking changed a spent output's record");

        // the unspent outputs went through a full round trip and keep their labels
        let first_record = wallet.output_record(first).expect("labeled output keeps its record");
        assert_eq!(first_record.label.as_deref(), Some("unspent label"));
        assert!(!wallet.is_locked(first) && !wallet.is_locked(second));
    }
}
