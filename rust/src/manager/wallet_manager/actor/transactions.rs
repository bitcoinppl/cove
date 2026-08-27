use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use act_zero::{ActorResult, AddrLike as _, Produces, WeakAddr, call, send};
use act_zero_ext::into_actor_result;
use bdk_wallet::{
    AddUtxoError, KeychainKind, LocalOutput, SignOptions, TxOrdering, Utxo, WeightedUtxo,
    chain::bitcoin::Psbt, error::CreateTxError,
};
use bitcoin::{
    Amount, FeeRate as BdkFeeRate, OutPoint, Transaction as BdkTransaction, TxIn, Txid,
    params::Params,
};
use cove_bdk::coin_selection::CoveDefaultCoinSelection;
use cove_device::keychain::Keychain;
use cove_types::{
    confirm::{AddressAndAmount, ConfirmDetails, ExtraItem, InputOutputDetails, SplitOutput},
    fees::{FeeRateOption, FeeRateOptionWithTotalFee, FeeRateOptions, FeeRateOptionsWithTotalFee},
    utxo::{UtxoList, UtxoType},
};
use cove_util::result_ext::ResultExt as _;
use tap::TapFallible as _;
use tracing::{debug, error, warn};

use crate::{
    manager::wallet_manager::{
        Error, WalletManagerBuildTxError, WalletManagerError, WalletManagerFeesError,
        WalletManagerReconcileMessage,
        actor::{SpendPolicy, WalletActor, current_wallet_unspent_outpoints_for_txid},
    },
    router::UnsignedPaymentMode,
    transaction::{FeeRate, Transaction, TransactionDetails, TransactionDetailsPresentation, TxId},
    wallet::Address,
    wallet_secret::WalletSecretExt as _,
};

use super::broadcast::{BroadcastTransactionError, broadcast_to_node_with_connection};

impl WalletActor {
    fn fee_from_insufficient_funds_needed(amount: Amount, needed: Amount) -> Result<Amount, Error> {
        needed.checked_sub(amount).ok_or_else(|| {
            WalletManagerError::FeesError(format!(
                "coin selection needed amount below send amount: needed_sats={}, amount_sats={}",
                needed.to_sat(),
                amount.to_sat()
            ))
        })
    }

    fn fee_option_with_total_fee(
        &mut self,
        option: FeeRateOption,
        amount: Amount,
        address: Address,
        spend_policy: &SpendPolicy,
    ) -> Result<FeeRateOptionWithTotalFee, Error> {
        let coin_selection = CoveDefaultCoinSelection::new(self.seed);
        let mut tx_builder = self.wallet.bdk.build_tx().coin_selection(coin_selection);

        spend_policy.apply(&mut tx_builder);
        tx_builder.ordering(TxOrdering::Untouched);
        tx_builder.add_recipient(address.script_pubkey(), amount);
        tx_builder.fee_rate(*option.fee_rate);

        let psbt = tx_builder.finish();
        let total_fee = match psbt {
            Ok(psbt) => {
                self.wallet.unreserve_tx_change_addresses(&psbt.unsigned_tx);
                psbt.fee().map_err(WalletManagerFeesError::from)?
            }
            Err(CreateTxError::CoinSelection(insufficient_funds)) => {
                Self::fee_from_insufficient_funds_needed(amount, insufficient_funds.needed)?
            }
            Err(error) => return Err(error.into()),
        };

        Ok(FeeRateOptionWithTotalFee::new(option, total_fee))
    }

    // Build a transaction but don't advance the change address index
    #[into_actor_result]
    pub async fn build_ephemeral_drain_tx(
        &mut self,
        address: Address,
        fee: FeeRate,
    ) -> Result<Psbt, Error> {
        self.ensure_ledger_ready_for_spend()?;

        debug!("build_ephemeral_drain_tx for fee rate {}", fee.sat_per_vb());
        let script_pubkey = address.script_pubkey();
        let spend_policy = self.automatic_spend_policy()?;
        let mut tx_builder = self.wallet.bdk.build_tx();

        spend_policy.apply(&mut tx_builder);
        tx_builder.drain_wallet().drain_to(script_pubkey).fee_rate(fee.into());
        let psbt = tx_builder.finish()?;
        self.wallet.unreserve_tx_change_addresses(&psbt.unsigned_tx);

        Ok(psbt)
    }

    /// Build a transaction
    #[into_actor_result]
    pub async fn build_tx(
        &mut self,
        amount: Amount,
        address: Address,
        fee_rate: impl Into<BdkFeeRate>,
    ) -> Result<Psbt, Error> {
        self.ensure_ledger_ready_for_spend()?;

        debug!("build_tx");
        let fee_rate = fee_rate.into();
        let script_pubkey = address.script_pubkey();

        let coin_selection = CoveDefaultCoinSelection::new(self.seed);
        let spend_policy = self.automatic_spend_policy()?;
        let mut tx_builder = self.wallet.bdk.build_tx().coin_selection(coin_selection);

        spend_policy.apply(&mut tx_builder);
        tx_builder.ordering(TxOrdering::Untouched);
        tx_builder.add_recipient(script_pubkey, amount);
        tx_builder.fee_rate(fee_rate);

        let psbt = tx_builder.finish()?;
        Ok(psbt)
    }

    /// Build a transaction but don't advance the change address index
    #[into_actor_result]
    pub async fn build_ephemeral_tx(
        &mut self,
        amount: Amount,
        address: Address,
        fee: impl Into<BdkFeeRate>,
    ) -> Result<Psbt, Error> {
        self.ensure_ledger_ready_for_spend()?;

        debug!("build_ephemeral_tx");
        let psbt = self.do_build_tx(amount, address, fee).await?;
        self.wallet.unreserve_tx_change_addresses(&psbt.unsigned_tx);
        Ok(psbt)
    }

    /// Build a transaction using only the given UTXOs
    #[into_actor_result]
    pub async fn build_manual_tx(
        &mut self,
        utxos: Vec<OutPoint>,
        total_amount: Amount,
        address: Address,
        fee_rate: impl Into<BdkFeeRate>,
    ) -> Result<Psbt, Error> {
        self.ensure_ledger_ready_for_spend()?;

        debug!("build_manual_tx: {total_amount:?}");

        let fee_rate = fee_rate.into();
        let send_amount = self.get_max_send_for_utxos(total_amount, &address, fee_rate, &utxos)?;

        let mut tx_builder = self.wallet.bdk.build_tx();
        tx_builder.add_utxos(&utxos)?;
        tx_builder.manually_selected_only();
        tx_builder.ordering(TxOrdering::Untouched);
        tx_builder.add_recipient(address.script_pubkey(), send_amount);
        tx_builder.fee_rate(fee_rate);

        let psbt = tx_builder.finish()?;
        Ok(psbt)
    }

    /// Build a manual transaction but don't advance the change address index
    #[into_actor_result]
    pub async fn build_manual_ephemeral_tx(
        &mut self,
        utxos: Vec<OutPoint>,
        amount: Amount,
        address: Address,
        fee: impl Into<BdkFeeRate>,
    ) -> Result<Psbt, Error> {
        self.ensure_ledger_ready_for_spend()?;

        debug!("build_manual_ephemeral_tx");
        let psbt = self.do_build_manual_tx(utxos, amount, address, fee).await?;
        self.wallet.unreserve_tx_change_addresses(&psbt.unsigned_tx);
        Ok(psbt)
    }

    // release reserved change addresses for a discarded unsigned transaction
    pub async fn cancel_txn(&mut self, txn: BdkTransaction) {
        self.wallet.unreserve_tx_change_addresses(&txn);
    }

    pub async fn list_unspent(&mut self) -> ActorResult<Result<Vec<LocalOutput>, Error>> {
        if let Err(error) = self.ensure_ledger_ready_for_spend() {
            return Produces::ok(Err(error));
        }

        Produces::ok(Ok(self.wallet.bdk.list_unspent().collect()))
    }

    pub async fn split_transaction_outputs(
        &mut self,
        outputs: Vec<AddressAndAmount>,
    ) -> ActorResult<SplitOutput> {
        let external = outputs
            .iter()
            .filter(|output| !self.wallet.bdk.is_mine(output.address.script_pubkey()))
            .cloned()
            .collect();

        let internal = outputs
            .iter()
            .filter(|output| self.wallet.bdk.is_mine(output.address.script_pubkey()))
            .cloned()
            .collect();

        Produces::ok(SplitOutput { external, internal })
    }

    #[into_actor_result]
    pub async fn fee_rate_options_with_total_fee(
        &mut self,
        fee_rate_options: FeeRateOptions,
        amount: Amount,
        address: Address,
    ) -> Result<FeeRateOptionsWithTotalFee, Error> {
        self.ensure_ledger_ready_for_spend()?;
        let spend_policy = self.automatic_spend_policy()?;

        let options = FeeRateOptionsWithTotalFee {
            fast: self.fee_option_with_total_fee(
                fee_rate_options.fast,
                amount,
                address.clone(),
                &spend_policy,
            )?,
            medium: self.fee_option_with_total_fee(
                fee_rate_options.medium,
                amount,
                address.clone(),
                &spend_policy,
            )?,
            slow: self.fee_option_with_total_fee(
                fee_rate_options.slow,
                amount,
                address.clone(),
                &spend_policy,
            )?,
            custom: None,
        };

        Ok(options)
    }

    #[into_actor_result]
    pub async fn fee_rate_options_with_total_fee_for_manual(
        &mut self,
        utxos: Arc<UtxoList>,
        fee_rate_options: FeeRateOptions,
        amount: Amount,
        address: Address,
    ) -> Result<FeeRateOptionsWithTotalFee, Error> {
        self.ensure_ledger_ready_for_spend()?;

        debug!("fee_rate_options_with_total_fee_for_manual");
        let fast_fee_rate = fee_rate_options.fast.fee_rate;
        let medium_fee_rate = fee_rate_options.medium.fee_rate;
        let slow_fee_rate = fee_rate_options.slow.fee_rate;

        let fast_psbt = self
            .do_build_manual_ephemeral_tx(
                utxos.clone().outpoints(),
                amount,
                address.clone(),
                fast_fee_rate,
            )
            .await?;

        let medium_psbt = self
            .do_build_manual_ephemeral_tx(
                utxos.clone().outpoints(),
                amount,
                address.clone(),
                medium_fee_rate,
            )
            .await?;

        let slow_psbt = self
            .do_build_manual_ephemeral_tx(
                utxos.clone().outpoints(),
                amount,
                address.clone(),
                slow_fee_rate,
            )
            .await?;

        let options = FeeRateOptionsWithTotalFee {
            fast: FeeRateOptionWithTotalFee::new(
                fee_rate_options.fast,
                fast_psbt.fee().map_err(WalletManagerFeesError::from)?,
            ),
            medium: FeeRateOptionWithTotalFee::new(
                fee_rate_options.medium,
                medium_psbt.fee().map_err(WalletManagerFeesError::from)?,
            ),
            slow: FeeRateOptionWithTotalFee::new(
                fee_rate_options.slow,
                slow_psbt.fee().map_err(WalletManagerFeesError::from)?,
            ),
            custom: None,
        };

        Ok(options)
    }

    #[into_actor_result]
    pub async fn fee_rate_options_with_total_fee_for_drain(
        &mut self,
        fee_rate_options: FeeRateOptions,
        address: Address,
    ) -> Result<FeeRateOptionsWithTotalFee, Error> {
        self.ensure_ledger_ready_for_spend()?;

        let options = fee_rate_options;

        let fast_fee_rate = options.fast.fee_rate;
        let fast_psbt: Psbt =
            self.do_build_ephemeral_drain_tx(address.clone(), fast_fee_rate).await?;

        let medium_fee_rate = options.medium.fee_rate;
        let medium_psbt: Psbt =
            self.do_build_ephemeral_drain_tx(address.clone(), medium_fee_rate).await?;

        let slow_fee_rate = options.slow.fee_rate;
        let slow_psbt: Psbt =
            self.do_build_ephemeral_drain_tx(address.clone(), slow_fee_rate).await?;

        let options_with_fee = FeeRateOptionsWithTotalFee {
            fast: FeeRateOptionWithTotalFee::new(
                options.fast,
                fast_psbt.fee().map_err(WalletManagerFeesError::from)?,
            ),
            medium: FeeRateOptionWithTotalFee::new(
                options.medium,
                medium_psbt.fee().map_err(WalletManagerFeesError::from)?,
            ),
            slow: FeeRateOptionWithTotalFee::new(
                options.slow,
                slow_psbt.fee().map_err(WalletManagerFeesError::from)?,
            ),
            custom: None,
        };

        Ok(options_with_fee)
    }

    #[into_actor_result]
    pub async fn get_confirm_details(
        &mut self,
        psbt: Psbt,
        fee_rate: BdkFeeRate,
    ) -> Result<ConfirmDetails, Error> {
        self.ensure_ledger_ready_for_spend()?;

        #[inline(always)]
        fn error(s: &str) -> WalletManagerError {
            WalletManagerError::GetConfirmDetailsError(s.to_string())
        }

        #[inline(always)]
        fn err_fmt(s: &str, err: impl std::fmt::Display) -> WalletManagerError {
            WalletManagerError::GetConfirmDetailsError(format!("{s}: {err}"))
        }

        let network = self.wallet.bdk.network();
        let external_outputs = psbt
            .unsigned_tx
            .output
            .iter()
            .filter(|output| !self.wallet.bdk.is_mine(output.script_pubkey.clone()))
            .collect::<Vec<&bitcoin::TxOut>>();

        if external_outputs.len() > 1 {
            return Err(error("multiple address to send to found, not currently supported"));
        }

        // if there is an external output, use that
        // otherwise this is a consolidation txn, sending to the same wallet so use the first output
        let output = external_outputs
            .first()
            .copied()
            .or_else(|| psbt.unsigned_tx.output.first())
            .ok_or_else(|| error("no addess to send to found"))?;

        let params = Params::from(network);
        let sending_to = bitcoin::Address::from_script(&output.script_pubkey, params)
            .map_err(|err| err_fmt("unable to get address from script", err))?;

        let output_amount = output.value;

        if output_amount == Amount::ZERO {
            return Err(error("zero amount"));
        }

        let fee = psbt.fee().map_err(|err| err_fmt("unable to get fee", err))?;

        let total_spent = output_amount
            .checked_add(fee)
            .ok_or_else(|| error("fee overflow, cannot calculate spending amount"))?;

        let psbt = cove_types::psbt::Psbt::from(psbt);
        let labels_db = self.db.labels.clone();

        let extras = psbt
            .utxos_iter()
            .map(|(tx_in, tx_out)| {
                let outpoint = &tx_in.previous_output;
                let utxo_type = self.wallet.bdk.get_utxo(*outpoint).map(|x| match x.keychain {
                    KeychainKind::External => UtxoType::Output,
                    KeychainKind::Internal => UtxoType::Change,
                });

                let address =
                    bitcoin::Address::from_script(&tx_out.script_pubkey, Params::from(network))
                        .ok();

                let label = labels_db
                    .get_txn_label_record(outpoint.txid)
                    .ok()
                    .flatten()
                    .map(|record| record.item.label)
                    .unwrap_or_else(|| match address {
                        Some(address) => labels_db
                            .get_address_record(address.into_unchecked())
                            .ok()
                            .flatten()
                            .and_then(|record| record.item.label),
                        None => None,
                    });

                let extra = ExtraItem::new(label, utxo_type);
                (&outpoint.txid, extra)
            })
            .collect();

        let more_details = InputOutputDetails::new_with_labels(&psbt, self.wallet.network, extras);
        let fee_percentage = fee.to_sat() * 100 / output_amount.to_sat();

        let details = ConfirmDetails {
            spending_amount: total_spent.into(),
            sending_amount: output_amount.into(),
            fee_total: fee.into(),
            fee_rate: fee_rate.into(),
            fee_percentage,
            sending_to: sending_to.into(),
            psbt,
            more_details,
        };

        Ok(details)
    }

    pub async fn initiate_payment(
        &mut self,
        psbt: Psbt,
        mode: UnsignedPaymentMode,
    ) -> ActorResult<Result<(), Error>> {
        if let Err(error) = self.ensure_ledger_ready_for_spend() {
            return Produces::ok(Err(error));
        }

        if let Err(error) = self.prepare_for_payment() {
            return Produces::ok(Err(error));
        }

        match mode {
            UnsignedPaymentMode::Standard => {
                let (_, transaction) = match self.do_sign_original_psbt(psbt).await {
                    Ok(result) => result,
                    Err(error) => return Produces::ok(Err(error)),
                };

                self.start_broadcast_transaction(transaction)
            }

            UnsignedPaymentMode::Payjoin { intent } => {
                let result = self.initiate_payjoin_payment(psbt, intent).await;
                Produces::ok(result)
            }
        }
    }

    pub(crate) async fn do_sign_original_psbt(
        &mut self,
        mut psbt: Psbt,
    ) -> Result<(Psbt, BdkTransaction), Error> {
        fn err(s: &str) -> Error {
            Error::SigningError(s.to_string())
        }

        let network = self.wallet.network;
        let secret = Keychain::global()
            .get_wallet_secret(&self.wallet.metadata.id)
            .tap_err(|error| error!("failed to get wallet secret: {error}"))?
            .ok_or_else(|| err("failed to get wallet secret"))?;
        let descriptors = secret.into_descriptors(network, self.wallet.metadata.address_type);

        let create_params = descriptors.into_create_params().network(network.into());

        // create a new temp wallet with the descriptors to sign
        let wallet = create_params
            .create_wallet_no_persist()
            .tap_err(|error| error!("failed to create wallet: {error}"))
            .map_err(|_| err("unable to sign"))?;

        let finalized = wallet
            .sign(&mut psbt, Default::default())
            .tap_err(|error| error!("failed to sign: {error}"))
            .map_err(|_| err("unable to sign"))?;

        if !finalized {
            return Err(err("transaction not finalized, unable to sign"));
        }

        let fallback_tx = psbt
            .clone()
            .extract_tx_fee_rate_limit()
            .tap_err(|error| error!("failed to extract transaction: {error}"))
            .map_err(|_| err("failed to extract transaction"))?;

        Ok((psbt, fallback_tx))
    }

    pub async fn broadcast_transaction(
        &mut self,
        transaction: BdkTransaction,
    ) -> ActorResult<Result<(), Error>> {
        if let Err(error) = self.ensure_ledger_ready_for_spend() {
            return Produces::ok(Err(error));
        }

        self.start_broadcast_transaction(transaction)
    }

    fn start_broadcast_transaction(
        &mut self,
        transaction: BdkTransaction,
    ) -> ActorResult<Result<(), Error>> {
        let connection = self.deferred_node_connection();
        let (reply, receiver) = futures::channel::oneshot::channel();

        self.addr.send_fut_with(|addr| async move {
            let result = broadcast_transaction_with_connection(addr, connection, transaction)
                .await
                .map_err(BroadcastTransactionError::into_error);
            let _ = reply.send(Produces::Value(result));
        });

        Ok(Produces::Deferred(receiver))
    }

    async fn apply_broadcast_transaction(
        &mut self,
        transaction: BdkTransaction,
    ) -> ActorResult<Result<(), Error>> {
        self.insert_broadcast_transaction(transaction).await;

        Produces::ok(Ok(()))
    }

    #[into_actor_result]
    #[allow(deprecated)] // SignOptions usage required by bdk_wallet API, no replacement yet
    pub async fn finalize_psbt(&mut self, psbt: Psbt) -> Result<bitcoin::Transaction, Error> {
        self.ensure_ledger_ready_for_spend()?;

        let mut psbt = psbt;

        let finalized = self
            .wallet
            .bdk
            .finalize_psbt(&mut psbt, SignOptions::default())
            .map_err_str(Error::PsbtFinalizeError)?;

        if !finalized {
            return Err(Error::PsbtFinalizeError("Failed to finalize PSBT".to_string()));
        }

        let tx = psbt.extract_tx().map_err(|e| {
            let err = format!("Failed to extract tx from PSBT: {e:?}");
            Error::PsbtFinalizeError(err)
        })?;

        Ok(tx)
    }

    /// Insert a broadcast transaction into the local wallet and update the UI
    async fn insert_broadcast_transaction(&mut self, transaction: BdkTransaction) {
        use WalletManagerReconcileMessage as Msg;

        let now =
            SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_else(|e| {
                warn!("System clock skew detected: {e}");
                0
            });

        let txid = transaction.compute_txid();

        // insert the unconfirmed transaction into the local wallet
        self.wallet.bdk.apply_unconfirmed_txs([(transaction, now)]);

        // persist the wallet to save the new transaction
        if let Err(error) = self.wallet.persist() {
            error!("Failed to persist wallet after inserting broadcast tx: {error}");
        }

        // send updated balance to UI
        let balance = self.wallet.balance();
        self.send(Msg::WalletBalanceChanged(balance.into()));

        // send updated transactions to UI
        let transactions = self.do_transactions().await;
        self.send(Msg::UpdatedTransactions(transactions));

        send!(self.addr.start_transaction_watcher(txid));
    }

    fn get_max_send_for_utxos(
        &mut self,
        total_amount: Amount,
        address: &Address,
        fee_rate: BdkFeeRate,
        utxos: &[OutPoint],
    ) -> Result<Amount, Error> {
        self.reject_locked_outpoints(utxos)?;

        let (utxo_total_amount, fee_estimate) = {
            let mut utxo_total_amount = Amount::ZERO;
            let mut total_fee_amount = Amount::ZERO;

            let weighted_utxos = self.get_weighted_utxos(utxos)?;

            for weighted_utxo in weighted_utxos {
                let weight = TxIn::default()
                    .segwit_weight()
                    .checked_add(weighted_utxo.satisfaction_weight)
                    .expect("`Weight` addition should not cause an integer overflow");

                utxo_total_amount += weighted_utxo.utxo.txout().value;
                total_fee_amount += fee_rate * weight;
            }

            (utxo_total_amount, total_fee_amount)
        };

        if total_amount > utxo_total_amount {
            return Err(Error::InsufficientFunds(format!(
                "custom amount {total_amount} is greater than the total amount available, total available: {utxo_total_amount}, fees: {fee_estimate}",
            )));
        }

        let fee = {
            let mut max_send_estimate =
                utxo_total_amount.checked_sub(fee_estimate).ok_or_else(|| {
                    Error::InsufficientFunds(format!(
                        "no enough funds to cover the fees, total available: {total_amount}, fees: {fee_estimate}",
                    ))
                })?;

            let recipient_dust_limit = address.script_pubkey().minimal_non_dust();
            let mut fee_psbt = None;
            while fee_psbt.is_none() {
                if max_send_estimate < recipient_dust_limit {
                    return Err(Error::OutputBelowDustLimit);
                }

                let mut tx_builder = self.wallet.bdk.build_tx();
                tx_builder.add_utxos(utxos)?;
                tx_builder.manually_selected_only();
                tx_builder.ordering(TxOrdering::Untouched);
                tx_builder.add_recipient(address.script_pubkey(), max_send_estimate);
                tx_builder.fee_rate(fee_rate);
                let current_fee_psbt = tx_builder.finish();

                match current_fee_psbt {
                    Ok(psbt) => {
                        fee_psbt = Some(psbt);
                    }
                    Err(CreateTxError::CoinSelection(result)) => {
                        let difference = result.needed - result.available;
                        let Some(reduced_estimate) = max_send_estimate.checked_sub(difference)
                        else {
                            return Err(Error::InsufficientFunds(format!(
                                "not enough funds to cover the fee shortfall, total available: {utxo_total_amount}, estimate: {max_send_estimate}, shortfall: {difference}",
                            )));
                        };

                        if reduced_estimate < recipient_dust_limit {
                            return Err(Error::OutputBelowDustLimit);
                        }

                        max_send_estimate = reduced_estimate;
                    }
                    Err(err) => return Err(err.into()),
                }
            }

            let fee_psbt = fee_psbt.expect("unwrapped in while");
            self.wallet.unreserve_tx_change_addresses(&fee_psbt.unsigned_tx);
            fee_psbt.fee().map_err(WalletManagerBuildTxError::from)?
        };

        let max_send_amount = utxo_total_amount.checked_sub(fee).ok_or_else(|| {
            Error::InsufficientFunds(format!(
                "no enough funds to cover the fees, total available: {total_amount}, fees: {fee}",
            ))
        })?;

        debug!("getting_max::send_amount: {max_send_amount:?}");
        if total_amount < max_send_amount {
            return Ok(total_amount);
        }

        Ok(max_send_amount)
    }

    fn get_weighted_utxos(
        &self,
        outpoints: &[bitcoin::OutPoint],
    ) -> Result<Vec<WeightedUtxo>, AddUtxoError> {
        outpoints
            .iter()
            .map(|outpoint| {
                self.wallet.bdk.get_utxo(*outpoint).ok_or(AddUtxoError::UnknownUtxo(*outpoint)).map(
                    |output| WeightedUtxo {
                        satisfaction_weight: self
                            .wallet
                            .bdk
                            .public_descriptor(output.keychain)
                            .max_weight_to_satisfy()
                            .unwrap(),
                        utxo: Utxo::Local(output),
                    },
                )
            })
            .collect()
    }

    pub(crate) fn current_wallet_unspent_outpoints_for_txid(&self, txid: Txid) -> Vec<OutPoint> {
        current_wallet_unspent_outpoints_for_txid(self.wallet.bdk.list_unspent(), txid)
    }

    pub(crate) fn transaction_for_tx_id(&self, tx_id: Txid) -> Result<Option<Transaction>, Error> {
        let Some(tx) = self.wallet.bdk.get_tx(tx_id) else {
            return Ok(None);
        };

        let sent_and_received = self.wallet.bdk.sent_and_received(&tx.tx_node.tx).into();
        let labels = self
            .db
            .labels
            .all_labels_for_txn(tx.tx_node.txid)
            .map_err_str(Error::TransactionDetailsError)?
            .into();

        Ok(Some(Transaction::new_with_labels(sent_and_received, tx, labels)))
    }

    pub(crate) fn transaction_details_for_tx_id(
        &self,
        tx_id: TxId,
    ) -> Result<TransactionDetails, Error> {
        let tx = self
            .wallet
            .bdk
            .get_tx(tx_id.0)
            .ok_or(Error::TransactionDetailsError("transaction not found".to_string()))?;

        let labels = self
            .db
            .labels
            .all_labels_for_txn(tx.tx_node.txid)
            .map_err_str(Error::TransactionDetailsError)?;
        TransactionDetails::try_new(&self.wallet.bdk, tx, labels.into())
            .map_err_str(Error::TransactionDetailsError)
    }

    pub(crate) fn transaction_details_presentation_for_tx_id(
        &mut self,
        tx_id: TxId,
    ) -> Result<TransactionDetailsPresentation, Error> {
        let details = self.transaction_details_for_tx_id(tx_id)?;
        let current_height = self.current_confirmation_height();

        Ok(TransactionDetailsPresentation::new(details, current_height))
    }

    fn current_confirmation_height(&mut self) -> u32 {
        self.last_height_fetched()
            .map(|(_, block_height)| block_height as u32)
            .unwrap_or_else(|| self.wallet.bdk.local_chain().tip().height())
    }
}

async fn broadcast_transaction_with_connection(
    addr: WeakAddr<WalletActor>,
    connection: Produces<Result<(), Error>>,
    transaction: BdkTransaction,
) -> Result<(), BroadcastTransactionError> {
    broadcast_to_node_with_connection(addr.clone(), connection, &transaction).await?;

    call!(addr.apply_broadcast_transaction(transaction))
        .await
        .map_err(|_| BroadcastTransactionError::PostBroadcastFailed(Error::ActorNotFound))?
        .map_err(BroadcastTransactionError::PostBroadcastFailed)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bdk_wallet::test_utils::{ReceiveTo, receive_output};
    use bitcoin::Amount;
    use parking_lot::RwLock;

    use super::{WalletActor, WalletManagerError};
    use crate::{
        database::wallet_data::{WalletDataDb, wallet_data_artifact_paths},
        manager::wallet_manager::{WalletScanStatus, WalletSnapshot},
        wallet::{Wallet, metadata::WalletMetadata},
    };

    #[test]
    fn preview_transaction_lookup_uses_in_memory_labels() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        crate::test_support::ensure_tokio_runtime();
        crate::database::test_support::init_test_database();
        crate::test_support::init_test_keychain();
        let metadata = WalletMetadata::preview_new();
        let wallet_data_paths = wallet_data_artifact_paths(&metadata.id);
        let mut wallet = Wallet::preview_new_wallet_with_metadata(metadata);
        let outpoint =
            receive_output(&mut wallet.bdk, Amount::from_sat(50_000), ReceiveTo::Mempool(1));
        let wallet_snapshot = Arc::new(RwLock::new(WalletSnapshot::from_wallet(&wallet)));
        let scan_status = Arc::new(RwLock::new(WalletScanStatus::Idle));
        let wallet_data_db =
            WalletDataDb::new_in_memory(wallet.id.clone()).expect("in-memory label database opens");
        let (reconciler, _) = flume::bounded(1);
        let actor = WalletActor::new_with_db(
            wallet,
            reconciler,
            scan_status,
            wallet_snapshot,
            wallet_data_db,
        );

        assert!(actor.transaction_for_tx_id(outpoint.txid).unwrap().is_some());
        assert!(wallet_data_paths.iter().all(|path| !path.exists()));
    }

    #[test]
    fn insufficient_funds_needed_amount_derives_fee() {
        let fee = WalletActor::fee_from_insufficient_funds_needed(
            Amount::from_sat(5_000),
            Amount::from_sat(5_156),
        )
        .expect("fee is derived");

        assert_eq!(fee, Amount::from_sat(156));
    }

    #[test]
    fn insufficient_funds_needed_below_amount_returns_error() {
        let error = WalletActor::fee_from_insufficient_funds_needed(
            Amount::from_sat(5_000),
            Amount::from_sat(4_999),
        )
        .expect_err("invalid needed amount is rejected");

        assert!(matches!(
            error,
            WalletManagerError::FeesError(message)
                if message.contains("needed amount below send amount")
        ));
    }
}
