//! Building a higher-fee replacement for a pending transaction this wallet sent

use std::{collections::BTreeSet, sync::Arc};

use act_zero_ext::into_actor_result;
use bdk_wallet::{KeychainKind, TxOrdering, chain::ChainPosition, chain::bitcoin::Psbt};
use bitcoin::{
    FeeRate as BdkFeeRate, OutPoint, ScriptBuf, Transaction as BdkTransaction, Txid, Weight,
};
use cove_bdk::coin_selection::CoveDefaultCoinSelection;
use cove_util::result_ext::ResultExt as _;
use tracing::{debug, warn};

use crate::{
    label_manager::LabelManager,
    manager::wallet_manager::actor::WalletActor,
    transaction::{
        TxId,
        fee_bump::{
            self, FeeBumpBlocker, FeeBumpConfirmContext, FeeBumpError, FeeBumpWallet,
            ReplaceableTransaction, ReplacementFunding,
        },
    },
};

/// A replacement's weight can change with the rate, when coin selection adds an input or drops
/// the change output, or a shrunk output drops below dust, so the minimum rate is searched for
/// a few times before giving up
const MINIMUM_FEE_RATE_ATTEMPTS: usize = 4;

/// A pending transaction that can be fee bumped, with the lowest rate a replacement can pay
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeeBumpTarget {
    pub original: ReplaceableTransaction,
    pub minimum_fee_rate: BdkFeeRate,
}

impl WalletActor {
    /// Whether `txid` can be fee bumped, and the lowest rate a replacement for it can pay
    ///
    /// Fails with a [`FeeBumpBlocker`] when the transaction can't be replaced at all, and with
    /// [`FeeBumpError::InsufficientFunds`] when even the lowest rate can't be paid
    #[into_actor_result]
    pub async fn fee_bump_target(&mut self, txid: Txid) -> Result<FeeBumpTarget, FeeBumpError> {
        let original = self.replaceable_transaction(txid)?;
        let minimum_fee_rate = self.minimum_replacement_fee_rate(&original)?;

        Ok(FeeBumpTarget { original, minimum_fee_rate })
    }

    /// Build a replacement for `txid` at `fee_rate` without reserving its change address, to
    /// show what a rate would cost
    #[into_actor_result]
    pub async fn build_ephemeral_fee_bump(
        &mut self,
        txid: Txid,
        fee_rate: BdkFeeRate,
    ) -> Result<Psbt, FeeBumpError> {
        let original = self.replaceable_transaction(txid)?;
        let psbt = self.build_ephemeral_replacement(&original, fee_rate)?;
        self.ensure_relays(&original, &psbt)?;

        Ok(psbt)
    }

    /// Build the replacement for `txid` at `fee_rate` that the user will sign
    #[into_actor_result]
    pub async fn build_fee_bump(
        &mut self,
        txid: Txid,
        fee_rate: BdkFeeRate,
    ) -> Result<Psbt, FeeBumpError> {
        debug!("build_fee_bump: {txid}");
        let original = self.replaceable_transaction(txid)?;
        let psbt = self.build_replacement(&original, fee_rate)?;

        if let Err(error) = self.ensure_relays(&original, &psbt) {
            self.wallet.unreserve_tx_change_addresses(&psbt.unsigned_tx);
            return Err(error);
        }

        Ok(psbt)
    }

    /// The pending payment `tx` takes the place of when it is a fee bump of one
    ///
    /// Asked by the confirm screen for any transaction it signs or broadcasts, so a replacement
    /// built here, signed on a hardware wallet or scanned back in shows as one alike
    #[into_actor_result]
    pub async fn fee_bump_context(
        &mut self,
        tx: BdkTransaction,
        fee: bitcoin::Amount,
    ) -> Option<FeeBumpConfirmContext> {
        fee_bump::replaced_payment(&self.wallet.bdk, &tx, fee)
    }

    /// The pending transaction `txid` if this wallet can replace it
    fn replaceable_transaction(&self, txid: Txid) -> Result<ReplaceableTransaction, FeeBumpError> {
        fee_bump::replaceable_transaction(&self.wallet.bdk, self.fee_bump_wallet(), txid)
    }

    pub(crate) fn fee_bump_wallet(&self) -> FeeBumpWallet {
        FeeBumpWallet {
            wallet_type: self.metadata.read().wallet_type,
            completed_initial_scan: self.completed_initial_scan(),
        }
    }

    /// The lowest rate a replacement can pay and still relay, see [`ReplaceableTransaction`]
    fn minimum_replacement_fee_rate(
        &mut self,
        original: &ReplaceableTransaction,
    ) -> Result<BdkFeeRate, FeeBumpError> {
        let mut fee_rate = original.starting_fee_rate();

        for _ in 0..MINIMUM_FEE_RATE_ATTEMPTS {
            let psbt = self.build_ephemeral_replacement(original, fee_rate)?;
            let weight = self.signed_weight_upper_bound(&psbt.unsigned_tx)?;
            let fee = psbt.fee().map_err_str(FeeBumpError::Build)?;

            if fee >= original.minimum_fee(weight) {
                return Ok(fee_rate);
            }

            let needed = original.minimum_fee_rate(weight);
            fee_rate = if needed > fee_rate {
                needed
            } else {
                BdkFeeRate::from_sat_per_kwu(fee_rate.to_sat_per_kwu() + 1)
            };
        }

        Err(FeeBumpError::Build("the minimum replacement fee rate did not settle".into()))
    }

    /// Refuses a replacement that pays less than the original's fee plus its own relay
    ///
    /// BDK only checks that the rate rises by one step, which a replacement that dropped its
    /// change output can meet while still paying too little in total for a node to accept it
    fn ensure_relays(
        &self,
        original: &ReplaceableTransaction,
        psbt: &Psbt,
    ) -> Result<(), FeeBumpError> {
        let weight = self.signed_weight_upper_bound(&psbt.unsigned_tx)?;
        let fee = psbt.fee().map_err_str(FeeBumpError::Build)?;

        if fee < original.minimum_fee(weight) {
            return Err(FeeBumpError::FeeRateTooLow { minimum: original.minimum_fee_rate(weight) });
        }

        Ok(())
    }

    fn build_ephemeral_replacement(
        &mut self,
        original: &ReplaceableTransaction,
        fee_rate: BdkFeeRate,
    ) -> Result<Psbt, FeeBumpError> {
        let psbt = self.build_replacement(original, fee_rate)?;
        self.wallet.unreserve_tx_change_addresses(&psbt.unsigned_tx);

        Ok(psbt)
    }

    /// Build a replacement that spends all of the original's inputs and pays the higher fee as
    /// [`ReplacementFunding`] says
    ///
    /// From change, it keeps the original's recipients and adds confirmed coins when the change
    /// can't cover the fee. From the amount, it spends only the original's inputs and drains them
    /// to its one output's script, so that output pays the fee, and a shrink below dust fails as
    /// [`FeeBumpError::InsufficientFunds`]
    fn build_replacement(
        &mut self,
        original: &ReplaceableTransaction,
        fee_rate: BdkFeeRate,
    ) -> Result<Psbt, FeeBumpError> {
        let original_tx =
            self.wallet.bdk.get_tx(original.txid).ok_or(FeeBumpError::NotFound)?.tx_node.tx;

        let original_inputs =
            original_tx.input.iter().map(|input| input.previous_output).collect::<Vec<_>>();

        // the original's inputs are spent already, so a lock on one only guards new spends
        let spend_policy = self.automatic_spend_policy()?.allowing(&original_inputs);

        let coin_selection = CoveDefaultCoinSelection::new(self.seed);
        let mut tx_builder =
            self.wallet.bdk.build_fee_bump(original.txid)?.coin_selection(coin_selection);

        spend_policy.apply(&mut tx_builder);
        tx_builder.ordering(TxOrdering::Untouched);
        tx_builder.fee_rate(fee_rate);

        // BDK's shrink pattern: with the recipient removed, the original's inputs, which BDK
        // already must spend, drain to its script after the fee
        if let ReplacementFunding::Amount { .. } = original.funding
            && let [recipient] = original_tx.output.as_slice()
        {
            tx_builder
                .set_recipients(Vec::new())
                .drain_to(recipient.script_pubkey.clone())
                .manually_selected_only();
        }

        Ok(tx_builder.finish()?)
    }

    /// The most the signed replacement can weigh, from each input's descriptor
    ///
    /// The unsigned transaction has no witnesses, so its own weight understates the fee a node
    /// counts. Every input is the wallet's own, checked before the replacement is built
    fn signed_weight_upper_bound(&self, tx: &BdkTransaction) -> Result<Weight, FeeBumpError> {
        let bdk = &self.wallet.bdk;

        // segwit marker and flag
        let mut weight = tx.weight() + Weight::from_wu(2);

        for input in &tx.input {
            let prevout = bdk
                .tx_graph()
                .get_txout(input.previous_output)
                .ok_or(FeeBumpBlocker::UnknownFee)?;

            let (keychain, _) = bdk
                .spk_index()
                .index_of_spk(prevout.script_pubkey.clone())
                .ok_or(FeeBumpBlocker::ForeignInputs)?;

            let satisfaction = bdk
                .public_descriptor(*keychain)
                .max_weight_to_satisfy()
                .map_err_str(FeeBumpError::Build)?;

            // the witness item count, as BDK's own estimate adds it
            weight += Weight::from_wu(1) + satisfaction;
        }

        Ok(weight)
    }
}

// MARK: after a replacement is broadcast

/// An unconfirmed wallet transaction that a just-broadcast one double-spends
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReplacedTransaction {
    pub(crate) txid: Txid,
    pub(crate) last_seen: u64,

    /// Whether the wallet still lists it, so the replacement must count as seen after it
    pub(crate) listed: bool,
}

impl ReplacedTransaction {
    /// When a transaction replacing `replaced` should count as seen, so that BDK, which keeps
    /// whichever double spend it saw last, lists it in their place
    pub(crate) fn seen_at(replaced: &[Self], now: u64) -> u64 {
        replaced
            .iter()
            .filter(|replaced| replaced.listed)
            .map(|replaced| replaced.last_seen.saturating_add(1))
            .fold(now, u64::max)
    }
}

impl WalletActor {
    /// The unconfirmed wallet transactions `tx` double-spends
    ///
    /// The node just accepted `tx`, so it holds none of them any more. Includes ones the wallet
    /// already dropped from its list, since a sync can find `tx` before its broadcast result
    /// arrives. Another device or a payjoin peer could replace `tx` in turn before its result
    /// arrives; the wallet then lists `tx` until the next sync stamps the node's transaction as
    /// seen later
    pub(crate) fn transactions_replaced_by(&self, tx: &BdkTransaction) -> Vec<ReplacedTransaction> {
        let bdk = &self.wallet.bdk;
        let graph = bdk.tx_graph();

        graph
            .direct_conflicts(tx)
            .filter_map(|(_, txid)| {
                let node = graph.get_tx_node(txid)?;
                let canonical = bdk.get_tx(txid).map(|canonical| canonical.chain_position);

                // a confirmed double spend would have made the node refuse `tx`
                if matches!(canonical, Some(ChainPosition::Confirmed { .. })) {
                    return None;
                }

                let last_seen = node.last_seen.unwrap_or_default();
                Some(ReplacedTransaction { txid, last_seen, listed: canonical.is_some() })
            })
            .collect()
    }

    /// Stops watching the transactions `replacement` replaced, and gives it their label and
    /// output locks
    ///
    /// Labels are keyed by txid and locks by outpoint, so without this a sped up payment would
    /// lose its label and its locked change would become spendable
    pub(crate) async fn hand_over_to_replacement(
        &mut self,
        replaced: &[ReplacedTransaction],
        replacement: &BdkTransaction,
    ) {
        for replaced in replaced {
            self.remove_watcher_for_txn(replaced.txid).await;
        }

        let txid = replacement.compute_txid();
        if let Err(error) = self.carry_label_to_replacement(replaced, replacement) {
            warn!("failed to carry the label of a replaced transaction to {txid}: {error}");
        }

        if let Err(error) = self.carry_locks_to_replacement(replaced, replacement) {
            warn!("failed to carry the output locks of a replaced transaction to {txid}: {error}");
        }
    }

    /// Locks each output of `replacement` that takes the place of a locked output of the latest
    /// transaction it replaced: one paying the same script, or its change when that change was
    /// locked
    ///
    /// A lock follows the coin rather than the payment, so a double spend paying someone else
    /// keeps the lock on its change too. BDK gives a replacement a fresh change address, so change
    /// is matched by keychain rather than by script
    fn carry_locks_to_replacement(
        &self,
        replaced: &[ReplacedTransaction],
        replacement: &BdkTransaction,
    ) -> Result<(), String> {
        let locked = self.db.labels.locked_output_outpoints().map_err(|error| error.to_string())?;
        let bdk = &self.wallet.bdk;
        let graph = bdk.tx_graph();
        let is_change = |script: &ScriptBuf| {
            bdk.derivation_of_spk(script.clone())
                .is_some_and(|(keychain, _)| keychain == KeychainKind::Internal)
        };

        // a payment sped up more than once leaves several originals, the latest one holds the
        // locks the user set last, which an older one's mustn't undo
        let Some(latest) = replaced.iter().max_by_key(|replaced| replaced.last_seen) else {
            return Ok(());
        };

        let Some(latest_tx) = graph.get_tx(latest.txid) else { return Ok(()) };
        let locked_scripts = (0_u32..)
            .zip(&latest_tx.output)
            .filter(|(vout, _)| locked.contains(&OutPoint { txid: latest.txid, vout: *vout }))
            .map(|(_, output)| output.script_pubkey.clone())
            .collect::<BTreeSet<_>>();

        let change_locked = locked_scripts.iter().any(is_change);
        let txid = replacement.compute_txid();
        let outpoints = (0_u32..)
            .zip(&replacement.output)
            .filter(|(_, output)| {
                locked_scripts.contains(&output.script_pubkey)
                    || (change_locked && is_change(&output.script_pubkey))
            })
            .map(|(vout, _)| OutPoint { txid, vout })
            .collect::<Vec<_>>();

        if outpoints.is_empty() {
            return Ok(());
        }

        LabelManager::new_with_db(self.db.clone())
            .set_output_spendability_for_outpoints(outpoints, false)
            .map_err(|error| error.to_string())
    }

    fn carry_label_to_replacement(
        &self,
        replaced: &[ReplacedTransaction],
        replacement: &BdkTransaction,
    ) -> Result<(), String> {
        let txid = replacement.compute_txid();
        let labels = &self.db.labels;
        if labels.get_txn_label_record(txid).map_err(|error| error.to_string())?.is_some() {
            return Ok(());
        }

        // only a fee bump of the same payment takes its label: a double spend from another
        // device can pay someone else entirely
        let bdk = &self.wallet.bdk;
        let recipients = fee_bump::outside_scripts(bdk, replacement);
        let graph = bdk.tx_graph();
        let same_payment = replaced.iter().filter(|replaced| {
            graph
                .get_tx(replaced.txid)
                .is_some_and(|tx| fee_bump::outside_scripts(bdk, &tx) == recipients)
        });

        // a payment sped up more than once leaves several originals, the latest one holds the
        // label the user saw last, or none when they deleted it, which an older one mustn't undo
        let Some(latest) = same_payment.max_by_key(|replaced| replaced.last_seen) else {
            return Ok(());
        };

        let record = labels.get_txn_label_record(latest.txid).map_err(|error| error.to_string())?;
        let Some(label) = record.map(|record| record.item).filter(|label| label.label.is_some())
        else {
            return Ok(());
        };

        let details = self
            .transaction_details_for_tx_id(TxId::from(txid))
            .map_err(|error| error.to_string())?;

        LabelManager::new_with_db(self.db.clone())
            .with_wallet_actor(self.addr.clone())
            .insert_or_update_labels_for_txn(
                Arc::new(details),
                label.label.unwrap_or_default(),
                label.origin,
            )
            .map_err(|error| error.to_string())
    }
}
