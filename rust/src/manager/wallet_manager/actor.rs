use crate::{
    database::{Database, wallet_data::WalletDataDb},
    historical_price_service::HistoricalPriceService,
    manager::wallet_manager::{Error, SendFlowErrorAlert, WalletManagerError},
    mnemonic,
    node::{
        Node,
        client::{NodeClient, NodeClientOptions},
        client_builder::NodeClientBuilder,
    },
    transaction::{ConfirmedTransaction, FeeRate, Transaction, TransactionDetails, TxId},
    transaction_watcher::TransactionWatcher,
    wallet::{
        Address, AddressInfo, Wallet, WalletAddressType, balance::Balance, metadata::BlockSizeLast,
    },
};
use act_zero::{runtimes::tokio::spawn_actor, *};
use act_zero_ext::into_actor_result;
use ahash::HashMap;
use bdk_wallet::{
    AddUtxoError, Utxo, WeightedUtxo,
    chain::{
        BlockId, TxGraph,
        bitcoin::Psbt,
        spk_client::{FullScanRequest, FullScanResponse, SyncRequest, SyncResponse},
    },
    error::CreateTxError,
};
// Note: SignOptions is marked deprecated in bdk_wallet 2.2.0 with a misleading message
// saying it moved to bitcoin::psbt, but no replacement exists there yet. bdk_wallet itself
// uses #![allow(deprecated)] and still requires SignOptions in its API. This will be resolved
// when bdk_wallet completes their signer refactoring.
#[allow(deprecated)]
use bdk_wallet::SignOptions;
use bdk_wallet::{KeychainKind, LocalOutput, TxOrdering};
use bitcoin::{Amount, FeeRate as BdkFeeRate, OutPoint, TxIn, Txid};
use bitcoin::{Transaction as BdkTransaction, params::Params};
use cove_bdk::coin_selection::CoveDefaultCoinSelection;
use cove_common::consts::{GAP_LIMIT, MIN_SEND_AMOUNT};
use cove_tokio_ext::FutureTimeoutExt as _;
use cove_types::{
    address::AddressInfoWithDerivation,
    confirm::{AddressAndAmount, ConfirmDetails, ExtraItem, InputOutputDetails, SplitOutput},
    fees::{FeeRateOptionWithTotalFee, FeeRateOptions, FeeRateOptionsWithTotalFee},
    utxo::{UtxoList, UtxoType},
};
use cove_util::result_ext::ResultExt as _;
use eyre::Result;
use flume::Sender;
use rand::Rng as _;
use std::{
    sync::Arc,
    time::{Duration, UNIX_EPOCH},
};
use tap::TapFallible as _;
use tracing::{debug, error, info, warn};

use self::mnemonic::{Mnemonic, MnemonicExt as _};

use super::{SingleOrMany, WalletManagerReconcileMessage};

#[derive(Debug)]
pub struct WalletActor {
    pub addr: WeakAddr<Self>,
    pub reconciler: Sender<SingleOrMany>,
    pub wallet: Wallet,
    pub node_client: Option<NodeClient>,

    pub db: WalletDataDb,
    pub state: ActorState,

    seed: u64,
    transaction_watchers: HashMap<Txid, Addr<TransactionWatcher>>,

    // cached values, source of truth is the redb database saved with wallet metadata
    last_scan_finished: Option<Duration>,
    last_height_fetched: Option<(Duration, usize)>,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum ActorState {
    Initial,
    PerformingIncrementalScan,
    PerformingFullScan(FullScanType),

    SyncScanComplete,

    FullScanComplete(FullScanType),

    /// incremental scan is performed after the expanded full scan
    IncrementalScanComplete,

    FailedFullScan(FullScanType),
    FailedIncrementalScan,
    FailedSyncScan,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum FullScanType {
    /// Initial scan scans for 20 addresses GAP_LIMIT
    Initial,
    /// Expanded scan scans for 150 addresses GAP_LIMIT
    Expanded,
}

impl FullScanType {
    fn stop_gap(&self) -> usize {
        match self {
            FullScanType::Initial => 20,
            FullScanType::Expanded => 150,
        }
    }
}

#[async_trait::async_trait]
impl Actor for WalletActor {
    async fn started(&mut self, addr: Addr<Self>) -> ActorResult<()> {
        self.addr = addr.downgrade();
        send!(addr.check_node_connection());
        Produces::ok(())
    }

    async fn error(&mut self, error: ActorError) -> bool {
        error!("WalletActor Error: {error:?}");
        let error_string = error.to_string();

        // an error occurred, that wasn't a wallet error, send unknown error
        let Some(error) = error.downcast::<Error>().ok().map(|e| *e) else {
            self.send(WalletManagerReconcileMessage::UnknownError(error_string));
            return false;
        };

        match error {
            Error::NodeConnectionFailed(error_string) => {
                self.send(WalletManagerReconcileMessage::NodeConnectionFailed(error_string));
            }

            Error::SignAndBroadcastError(_) => {
                self.send(WalletManagerReconcileMessage::SendFlowError(
                    SendFlowErrorAlert::SignAndBroadcast(error.to_string()),
                ));
            }

            Error::GetConfirmDetailsError(_) => {
                self.send(WalletManagerReconcileMessage::SendFlowError(
                    SendFlowErrorAlert::ConfirmDetails(error.to_string()),
                ));
            }

            _ => {
                self.send(WalletManagerReconcileMessage::WalletError(error));
            }
        };

        false
    }
}

impl WalletActor {
    pub fn new(wallet: Wallet, reconciler: Sender<SingleOrMany>) -> Self {
        let db = WalletDataDb::new_or_existing(wallet.id.clone());
        let seed = rand::rng().random();

        Self {
            addr: Default::default(),
            reconciler,
            seed,
            wallet,
            node_client: None,
            last_scan_finished: None,
            last_height_fetched: None,
            state: ActorState::Initial,
            transaction_watchers: HashMap::default(),
            db,
        }
    }

    pub async fn balance(&mut self) -> ActorResult<Balance> {
        let balance = self.wallet.balance();
        Produces::ok(balance)
    }

    // Build a transaction but don't advance the change address index
    #[into_actor_result]
    pub async fn build_ephemeral_drain_tx(
        &mut self,
        address: Address,
        fee: FeeRate,
    ) -> Result<Psbt, Error> {
        debug!("build_ephemeral_drain_tx for fee rate {}", fee.sat_per_vb());
        let script_pubkey = address.script_pubkey();
        let mut tx_builder = self.wallet.bdk.build_tx();

        tx_builder.drain_wallet().drain_to(script_pubkey).fee_rate(fee.into());
        let psbt = tx_builder.finish().map_err_str(Error::BuildTxError)?;
        self.wallet.bdk.cancel_tx(&psbt.unsigned_tx);

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
        debug!("build_tx");
        let fee_rate = fee_rate.into();
        let script_pubkey = address.script_pubkey();

        let coin_selection = CoveDefaultCoinSelection::new(self.seed);
        let mut tx_builder = self.wallet.bdk.build_tx().coin_selection(coin_selection);

        tx_builder.ordering(TxOrdering::Untouched);
        tx_builder.add_recipient(script_pubkey, amount);
        tx_builder.fee_rate(fee_rate);

        let psbt = tx_builder.finish().map_err_str(Error::BuildTxError)?;
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
        debug!("build_ephemeral_tx");
        let psbt = self.do_build_tx(amount, address, fee).await?;
        self.wallet.bdk.cancel_tx(&psbt.unsigned_tx);
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
        debug!("build_manual_tx: {total_amount:?}");

        let fee_rate = fee_rate.into();
        let send_amount = self.get_max_send_for_utxos(total_amount, &address, fee_rate, &utxos)?;

        let mut tx_builder = self.wallet.bdk.build_tx();
        tx_builder.add_utxos(&utxos).map_err_str(Error::AddUtxosError)?;
        tx_builder.manually_selected_only();
        tx_builder.ordering(TxOrdering::Untouched);
        tx_builder.add_recipient(address.script_pubkey(), send_amount);
        tx_builder.fee_rate(fee_rate);

        let psbt = tx_builder.finish().map_err_str(Error::BuildTxError)?;
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
        debug!("build_manual_ephemeral_tx");
        let psbt = self.do_build_manual_tx(utxos, amount, address, fee).await?;
        self.wallet.bdk.cancel_tx(&psbt.unsigned_tx);
        Ok(psbt)
    }

    // cancel a transaction, reset the address & change address index
    pub async fn cancel_txn(&mut self, txn: BdkTransaction) {
        self.wallet.bdk.cancel_tx(&txn)
    }

    pub async fn list_unspent(&mut self) -> ActorResult<Vec<LocalOutput>> {
        Produces::ok(self.wallet.bdk.list_unspent().collect())
    }

    #[act_zero_ext::into_actor_result]
    pub async fn transactions(&mut self) -> Vec<Transaction> {
        let zero = Amount::ZERO.into();

        let mut transactions = self
            .wallet
            .bdk
            .transactions()
            .map(|tx| {
                let sent_and_received = self.wallet.bdk.sent_and_received(&tx.tx_node.tx).into();
                (tx, sent_and_received)
            })
            .map(|(tx, sent_and_received)| Transaction::new(&self.wallet.id, sent_and_received, tx))
            .filter(|tx| tx.sent_and_received().amount() > zero)
            .inspect(|tx| {
                if let Transaction::Unconfirmed(unconfirmed) = &tx {
                    send!(self.addr.start_transaction_watcher(unconfirmed.txid.0));
                };
            })
            .collect::<Vec<Transaction>>();

        transactions.sort_unstable_by(|a, b| a.cmp(b).reverse());
        transactions
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
        let fast_fee_rate = fee_rate_options.fast.fee_rate;
        let medium_fee_rate = fee_rate_options.medium.fee_rate;
        let slow_fee_rate = fee_rate_options.slow.fee_rate;

        let fast_psbt = self.do_build_ephemeral_tx(amount, address.clone(), fast_fee_rate).await?;

        let medium_psbt =
            self.do_build_ephemeral_tx(amount, address.clone(), medium_fee_rate).await?;

        let slow_psbt = self.do_build_ephemeral_tx(amount, address.clone(), slow_fee_rate).await?;

        let options = FeeRateOptionsWithTotalFee {
            fast: FeeRateOptionWithTotalFee::new(
                fee_rate_options.fast,
                fast_psbt.fee().map_err_str(Error::FeesError)?,
            ),
            medium: FeeRateOptionWithTotalFee::new(
                fee_rate_options.medium,
                medium_psbt.fee().map_err_str(Error::FeesError)?,
            ),
            slow: FeeRateOptionWithTotalFee::new(
                fee_rate_options.slow,
                slow_psbt.fee().map_err_str(Error::FeesError)?,
            ),
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
                fast_psbt.fee().map_err_str(Error::FeesError)?,
            ),
            medium: FeeRateOptionWithTotalFee::new(
                fee_rate_options.medium,
                medium_psbt.fee().map_err_str(Error::FeesError)?,
            ),
            slow: FeeRateOptionWithTotalFee::new(
                fee_rate_options.slow,
                slow_psbt.fee().map_err_str(Error::FeesError)?,
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
                fast_psbt.fee().map_err_str(Error::FeesError)?,
            ),
            medium: FeeRateOptionWithTotalFee::new(
                options.medium,
                medium_psbt.fee().map_err_str(Error::FeesError)?,
            ),
            slow: FeeRateOptionWithTotalFee::new(
                options.slow,
                slow_psbt.fee().map_err_str(Error::FeesError)?,
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
            .cloned()
            .or_else(|| psbt.unsigned_tx.output.first())
            .ok_or_else(|| error("no addess to send to found"))?;

        let params = Params::from(network);
        let sending_to = bitcoin::Address::from_script(&output.script_pubkey, params)
            .map_err(|err| err_fmt("unable to get address from script", err))?;

        let sending_amount = output.value;

        if sending_amount == Amount::ZERO {
            return Err(error("zero amount"));
        }

        let fee = psbt.fee().map_err(|err| err_fmt("unable to get fee", err))?;

        let spending_amount = sending_amount
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

        let more_details = InputOutputDetails::new_with_labels(&psbt, network.into(), extras);
        let fee_percentage = fee.to_sat() * 100 / sending_amount.to_sat();

        let details = ConfirmDetails {
            spending_amount: spending_amount.into(),
            sending_amount: sending_amount.into(),
            fee_total: fee.into(),
            fee_rate: fee_rate.into(),
            fee_percentage,
            sending_to: sending_to.into(),
            psbt,
            more_details,
        };

        Ok(details)
    }

    pub async fn sign_and_broadcast_transaction(
        &mut self,
        psbt: Psbt,
    ) -> ActorResult<Result<(), Error>> {
        let result = self.do_sign_and_broadcast_transaction(psbt).await;
        Produces::ok(result)
    }

    async fn do_sign_and_broadcast_transaction(&mut self, mut psbt: Psbt) -> Result<(), Error> {
        fn err(s: &str) -> Error {
            Error::SignAndBroadcastError(s.to_string())
        }

        let network = self.wallet.network;
        let mnemonic = Mnemonic::try_from_id(&self.wallet.metadata.id)
            .tap_err(|error| error!("failed to get mnemonic for wallet: {error}"))
            .map_err(|_| err("failed to get mnemonic for wallet"))?;

        let descriptors =
            mnemonic.into_descriptors(None, network, self.wallet.metadata.address_type);

        let create_params = descriptors.into_create_params().network(network.into());

        // create a new temp wallet with the descriptors
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

        let transaction = psbt
            .extract_tx()
            .tap_err(|error| error!("failed to extract transaction: {error}"))
            .map_err(|_| err("failed to extract transaction"))?;

        self.do_broadcast_transaction(transaction).await?;

        Ok(())
    }

    #[into_actor_result]
    pub async fn broadcast_transaction(
        &mut self,
        transaction: BdkTransaction,
    ) -> Result<(), Error> {
        self.check_node_connection().await.map_err(|error| {
            let error_string =
                format!("failed to broadcast transaction, unable to connect to node: {error:?}");
            Error::SignAndBroadcastError(error_string)
        })?;

        self.node_client()
            .await
            .map_err(|_| {
                Error::SignAndBroadcastError(
                    "failed to broadcast transaction, could not get node client, try again"
                        .to_string(),
                )
            })?
            .broadcast_transaction(transaction)
            .await
            .map_err(|error| {
                let error_string = format!("failed to broadcast transaction, try again: {error:?}");
                Error::SignAndBroadcastError(error_string)
            })?;

        Ok(())
    }

    #[into_actor_result]
    #[allow(deprecated)] // SignOptions usage required by bdk_wallet API, no replacement yet
    pub async fn finalize_psbt(&mut self, psbt: Psbt) -> Result<bitcoin::Transaction, Error> {
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

    pub async fn address_at(&mut self, index: u32) -> ActorResult<AddressInfo> {
        let address = self.wallet.bdk.peek_address(KeychainKind::External, index);
        Produces::ok(address.into())
    }

    pub async fn next_address(&mut self) -> ActorResult<AddressInfoWithDerivation> {
        let address = self.wallet.get_next_address()?;
        Produces::ok(address)
    }

    #[into_actor_result]
    pub async fn check_node_connection(&mut self) {
        let node = Database::global().global_config.selected_node();

        let reconciler = self.reconciler.clone();
        self.addr.send_fut(async move {
            if let Err(error) = check_node_connection_inner(&node).await {
                let _ = reconciler
                    .send(WalletManagerReconcileMessage::NodeConnectionFailed(error).into());
            }
        });
    }

    pub async fn wallet_scan_and_notify(&mut self, force_scan: bool) -> ActorResult<()> {
        use WalletManagerReconcileMessage as Msg;
        debug!("wallet_scan_and_notify");

        // get the initial balance and transactions
        {
            let initial_balance =
                self.balance().await?.await.map_err_str(Error::WalletBalanceError)?;

            self.send(Msg::WalletBalanceChanged(initial_balance.into()));

            let initial_transactions =
                self.transactions().await?.await.map_err_str(Error::TransactionsRetrievalError)?;

            self.send(Msg::AvailableTransactions(initial_transactions))
        }

        // start the wallet scan in a background task
        self.start_wallet_scan_in_task(force_scan)
            .await?
            .await
            .map_err_str(Error::WalletScanError)?;

        Produces::ok(())
    }

    pub async fn start_wallet_scan_in_task(&mut self, force_scan: bool) -> ActorResult<()> {
        debug!("start_wallet_scan");

        if let Some(last_scan) = self.last_scan_finished()
            && elapsed_secs_since(last_scan) < 15
            && !force_scan
        {
            debug!("skipping wallet scan, last scan was less than 15 seconds ago");
            return Produces::ok(());
        }

        // check the node connection, and send frontend the error if it fails
        self.check_node_connection().await?;

        // perform that scanning in a background task
        let addr = self.addr.clone();
        if self.wallet.metadata.internal.performed_full_scan_at.is_some() {
            send!(addr.perform_incremental_scan());
        } else {
            send!(addr.perform_full_scan());
        }

        Produces::ok(())
    }

    pub async fn get_height(&mut self, force: bool) -> ActorResult<usize> {
        if let Some((last_height_fetched, block_height)) = self.last_height_fetched() {
            let elapsed = elapsed_secs_since(last_height_fetched);
            if !force && elapsed < 120 {
                // if less than 25 seconds return the height, without updating
                if elapsed < 25 {
                    return Produces::ok(block_height);
                }

                // if more than a minute return height immediately, but update the height in the background
                send!(self.addr.update_height());
                return Produces::ok(block_height);
            }
        }

        // update the height and return the new height
        let block_height = self.update_height().await?.await?;
        Produces::ok(block_height)
    }

    pub async fn switch_mnemonic_to_new_address_type(
        &mut self,
        address_type: WalletAddressType,
    ) -> ActorResult<()> {
        debug!("actor switch mnemonic wallet");

        self.wallet.switch_mnemonic_to_new_address_type(address_type)?;

        Produces::ok(())
    }

    pub async fn switch_descriptor_to_new_address_type(
        &mut self,
        descriptors: pubport::descriptor::Descriptors,
        address_type: WalletAddressType,
    ) -> ActorResult<()> {
        debug!("actor switch pubkey descriptor wallet");

        self.wallet.switch_descriptor_to_new_address_type(descriptors, address_type)?;

        Produces::ok(())
    }

    async fn update_height(&mut self) -> ActorResult<usize> {
        debug!("actor update_height");
        self.check_node_connection().await?;
        let node_client = self.node_client().await?;
        let block_height = node_client.get_height().await.map_err(|_| Error::GetHeightError)?;

        self.save_last_height_fetched(block_height);
        Produces::ok(block_height)
    }

    async fn update_block_id(&mut self) -> eyre::Result<BlockId> {
        debug!("actor update_block_id");
        if self.check_node_connection().await.is_err() {
            return Err(eyre::eyre!("node connection failed"));
        }

        let node_client = self.node_client().await?;
        let block_id = node_client.get_block_id().await.map_err(|_| Error::GetHeightError)?;
        self.save_last_height_fetched(block_id.height as usize);
        Ok(block_id)
    }

    #[into_actor_result]
    pub async fn txns_with_prices(&mut self) -> Result<Vec<(ConfirmedTransaction, Option<f32>)>> {
        let network = self.wallet.network;
        let fiat_currency = Database::global().global_config.fiat_currency().unwrap_or_default();

        let confirmed_transactions = self
            .do_transactions()
            .await
            .into_iter()
            .filter_map(|tx| match tx {
                Transaction::Confirmed(confirmed) => Some(confirmed),
                Transaction::Unconfirmed(_) => None,
            })
            .map(Arc::unwrap_or_clone)
            .collect::<Vec<_>>();

        let historical_prices_service = HistoricalPriceService::new();
        let txns_with_prices = historical_prices_service
            .get_prices_for_transactions(network, fiat_currency, confirmed_transactions)
            .await
            .map_err_str(Error::GetHistoricalPricesError)?;

        Ok(txns_with_prices)
    }

    pub async fn transaction_details(&mut self, tx_id: TxId) -> ActorResult<TransactionDetails> {
        let tx = self
            .wallet
            .bdk
            .get_tx(tx_id.0)
            .ok_or(Error::TransactionDetailsError("transaction not found".to_string()))?;

        let labels = self.db.labels.all_labels_for_txn(tx.tx_node.txid)?;
        let details = TransactionDetails::try_new(&self.wallet.bdk, tx, labels.into())
            .map_err_str(Error::TransactionDetailsError)?;

        Produces::ok(details)
    }

    pub async fn start_transaction_watcher(&mut self, tx_id: Txid) -> ActorResult<()> {
        debug!("start_transaction_watcher for txn: {tx_id}");
        if self.transaction_watchers.contains_key(&tx_id) {
            warn!("transaction watcher already exists for txn: {tx_id}");
            return Produces::ok(());
        }

        let network = self.wallet.network;
        let node = Database::global().global_config.selected_node();
        let options = NodeClientOptions { batch_size: 1 };
        let client_builder = NodeClientBuilder { node, options };

        let watcher = TransactionWatcher::new(self.addr.clone(), tx_id, client_builder, network);
        let addr = spawn_actor(watcher);

        self.transaction_watchers.insert(tx_id, addr);

        Produces::ok(())
    }

    /// will remove the transaction watcher if it exists and
    /// perform a sync scan and send the transactions to the frontend
    pub async fn mark_transaction_found(&mut self, tx_id: Txid) -> ActorResult<()> {
        info!("marking transaction found: {tx_id}");

        // update the height
        self.update_block_id().await?;

        // update the height and perform sync scan which will update the transactions
        self.perform_scan_for_single_tx_id(tx_id).await?;

        // wait 30 seconds run the scan again and then remove the watcher
        // sanity check to make sure the transaction was picked up by the wallet
        // and no extra watchers were created in the meantime
        let addr = self.addr.clone();
        self.addr.send_fut(async move {
            tokio::time::sleep(Duration::from_secs(30)).await;
            send!(addr.perform_scan_for_single_tx_id(tx_id));
            send!(addr.remove_watcher_for_txn(tx_id));
        });

        Produces::ok(())
    }

    pub async fn stop_all_scans(&mut self) {
        debug!("stop_all_scans");
        self.transaction_watchers = HashMap::default();
        // TODO: stop the wallet scans too, need to save the task handle when we start the scan
    }

    async fn remove_watcher_for_txn(&mut self, tx_id: Txid) {
        debug!("removing watcher for txn: {tx_id}");
        self.transaction_watchers.remove(&tx_id);
    }

    async fn perform_scan_for_single_tx_id(&mut self, tx_id: Txid) -> ActorResult<()> {
        let start = UNIX_EPOCH.elapsed().unwrap().as_secs();
        let _ = self.update_height().await?.await;

        let chain_tip = self.wallet.bdk.local_chain().tip();
        let sync_request_builder = SyncRequest::builder().txids(vec![tx_id]).chain_tip(chain_tip);

        let sync_request = sync_request_builder.build();

        let node_client = self.node_client().await?.clone();
        let graph = self.wallet.bdk.tx_graph().clone();

        let now = UNIX_EPOCH.elapsed().unwrap().as_secs();
        debug!("done scan for spk in {}s", now - start);

        let addr = self.addr.clone();
        self.addr.send_fut(async move {
            let scan_result = node_client.sync(&graph, sync_request).await;

            let now = UNIX_EPOCH.elapsed().unwrap().as_secs();
            debug!("done single txn id sync scan in {}s", now - start);

            // save updated txns and send to frontend
            send!(addr.update_sync_state_and_send_transactions(scan_result));
        });

        Produces::ok(())
    }

    // perform full scan in 2 steps:
    // 1. do a full scan of the first 20 addresses, return results
    // 2. do a full scan of the next 150 addresses, return results
    async fn perform_full_scan(&mut self) -> ActorResult<()> {
        self.maybe_perform_initial_full_scan().await?;
        Produces::ok(())
    }

    // when a wallet is first opened, we need to scan for its addresses, but we want the
    // initial scan to be fast, so we can have transactions show up in the UI quickly
    // so we do a full scan of only the first 20 addresses, initially
    async fn maybe_perform_initial_full_scan(&mut self) -> ActorResult<()> {
        if self.state != ActorState::Initial {
            debug!("already performing scanning or scanned skipping ({:?})", self.state);

            return Produces::ok(());
        }

        debug!("starting initial full scan");
        self.reconciler.send(WalletManagerReconcileMessage::StartedInitialFullScan.into()).unwrap();

        // scan happens in the background, state update afterwards
        self.state = ActorState::PerformingFullScan(FullScanType::Initial);
        self.perform_initial_full_scan().await?;

        Produces::ok(())
    }

    /// Get the max amount that can be sent for the given utxos
    /// The total amount max is the total value of the UTXOs.
    /// If the total amount is less than the UTXO total amount we will have a change output as well.
    fn get_max_send_for_utxos(
        &mut self,
        total_amount: Amount,
        address: &Address,
        fee_rate: impl Into<BdkFeeRate>,
        utxos: &[bitcoin::OutPoint],
    ) -> Result<Amount, Error> {
        let fee_rate = fee_rate.into();

        let (utxo_total_amount, fee_estimate) = {
            let mut utxo_total_amount = Amount::ZERO;
            let mut total_fee_amount = Amount::ZERO;

            let weighted_utxos =
                self.get_weighted_utxos(utxos).map_err_str(Error::AddUtxosError)?;

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
        };

        let fee = {
            let mut max_send_estimate =
                utxo_total_amount.checked_sub(fee_estimate).ok_or_else(|| {
                    Error::InsufficientFunds(format!(
                        "no enough funds to cover the fees, total available: {total_amount}, fees: {fee_estimate}",
                    ))
                })?;

            let mut fee_psbt = None;
            while fee_psbt.is_none() {
                if max_send_estimate < MIN_SEND_AMOUNT {
                    return Err(Error::InsufficientFunds(format!(
                        "no enough funds to cover the fees, total available: {total_amount}, fees: {fee_estimate}",
                    )));
                }

                let mut tx_builder = self.wallet.bdk.build_tx();
                tx_builder.add_utxos(utxos).map_err_str(Error::AddUtxosError)?;
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
                        max_send_estimate -= difference;
                    }
                    Err(err) => return Err(Error::BuildTxError(err.to_string())),
                }
            }

            let fee_psbt = fee_psbt.expect("unwrapped in while");
            self.wallet.bdk.cancel_tx(&fee_psbt.unsigned_tx);
            fee_psbt.fee().map_err_str(Error::BuildTxError)?
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

    // after the initial full scan is complete, do a much for comprehensive scan of the wallet
    // this is slower, but we want to be able to see all transactions in the UI, so scan the next
    // 150 addresses
    async fn maybe_perform_expanded_full_scan(&mut self) -> ActorResult<()> {
        if self.state == ActorState::FullScanComplete(FullScanType::Expanded)
            || self.state == ActorState::IncrementalScanComplete
        {
            debug!("already scanned skipping expanded full scan ({:?})", self.state);
            return Produces::ok(());
        }

        debug!("starting expanded full scan");
        let txns = self.transactions().await?.await?;

        self.reconciler
            .send(WalletManagerReconcileMessage::StartedExpandedFullScan(txns).into())
            .unwrap();

        // scan happens in the background, state update afterwards
        self.state = ActorState::PerformingFullScan(FullScanType::Expanded);
        send!(self.addr.perform_expanded_full_scan());

        Produces::ok(())
    }

    async fn perform_initial_full_scan(&mut self) -> ActorResult<()> {
        debug!("perform_initial_full_scan");
        static FULL_SCAN_TYPE: FullScanType = FullScanType::Initial;

        let (full_scan_request, graph, node_client) = self.get_for_full_scan().await?;

        let addr = self.addr.clone();
        self.addr.send_fut(async move {
            let start = UNIX_EPOCH.elapsed().unwrap().as_secs();

            let full_scan_result = node_client
                .start_wallet_scan(&graph, full_scan_request, FULL_SCAN_TYPE.stop_gap())
                .await;

            let now = UNIX_EPOCH.elapsed().unwrap().as_secs();
            debug!("[initial] done initial full scan in {}s", now - start);

            // update wallet state
            let _ = call!(addr.handle_full_scan_complete(full_scan_result, FULL_SCAN_TYPE)).await;

            // perform next scan
            send!(addr.maybe_perform_expanded_full_scan());
        });

        Produces::ok(())
    }

    async fn perform_expanded_full_scan(&mut self) -> ActorResult<()> {
        debug!("perform_expanded_full_scan");
        static FULL_SCAN_TYPE: FullScanType = FullScanType::Expanded;

        let (full_scan_request, graph, node_client) = self.get_for_full_scan().await?;

        let addr = self.addr.clone();
        self.addr.send_fut(async move {
            let start = UNIX_EPOCH.elapsed().unwrap().as_secs();

            let full_scan_result = node_client
                .start_wallet_scan(&graph, full_scan_request, FULL_SCAN_TYPE.stop_gap())
                .await;

            let now = UNIX_EPOCH.elapsed().unwrap().as_secs();
            debug!("[expanded] done expanded full scan in {}s", now - start);

            // update wallet state
            send!(addr.handle_full_scan_complete(full_scan_result, FULL_SCAN_TYPE));
        });

        Produces::ok(())
    }

    async fn get_for_full_scan(
        &mut self,
    ) -> Result<(FullScanRequest<KeychainKind>, TxGraph, NodeClient)> {
        let node_client = self.node_client().await?.clone();

        let full_scan_request = self.wallet.bdk.start_full_scan().build();
        let graph = self.wallet.bdk.tx_graph().clone();

        Ok((full_scan_request, graph, node_client))
    }

    async fn perform_incremental_scan(&mut self) -> ActorResult<()> {
        debug!("starting incremental scan");

        self.state = ActorState::PerformingIncrementalScan;
        let start = UNIX_EPOCH.elapsed().unwrap().as_secs();

        let node_client = self.node_client().await?.clone();

        let full_scan_request = self.wallet.bdk.start_full_scan().build();
        let graph = self.wallet.bdk.tx_graph().clone();

        let addr = self.addr.clone();
        self.addr.send_fut(async move {
            let scan_result =
                node_client.start_wallet_scan(&graph, full_scan_request, GAP_LIMIT as usize).await;

            let now = UNIX_EPOCH.elapsed().unwrap().as_secs();
            debug!("done incremental scan in {}s", now - start);

            // update wallet state
            send!(addr.handle_incremental_scan_complete(scan_result));
        });

        Produces::ok(())
    }

    async fn handle_full_scan_complete(
        &mut self,
        full_scan_result: Result<FullScanResponse<KeychainKind>, crate::node::client::Error>,
        full_scan_type: FullScanType,
    ) -> ActorResult<()> {
        debug!("applying full scan result for {full_scan_type:?}");

        match full_scan_result {
            Ok(full_scan_result) => {
                self.wallet.bdk.apply_update(full_scan_result)?;
                self.wallet.persist()?;
            }
            Err(error) => {
                self.state = ActorState::FailedFullScan(full_scan_type);
                return Err(error.into());
            }
        }

        // only mark as scan complete when the expanded full scan is complete
        if full_scan_type == FullScanType::Expanded {
            let now = jiff::Timestamp::now().as_second() as u64;
            self.wallet.metadata.internal.performed_full_scan_at = Some(now);
            Database::global().wallets.update_internal_metadata(&self.wallet.metadata)?;
        }

        // always update the last scan finished time
        self.save_last_scan_finished();
        self.notify_scan_complete().await?;

        // update the state
        self.state = ActorState::FullScanComplete(full_scan_type);

        Produces::ok(())
    }

    async fn handle_incremental_scan_complete(
        &mut self,
        scan_result: Result<FullScanResponse<KeychainKind>, crate::node::client::Error>,
    ) -> ActorResult<()> {
        if scan_result.is_err() {
            self.state = ActorState::FailedIncrementalScan;
        }

        let sync_result = scan_result?;
        self.wallet.bdk.apply_update(sync_result)?;
        self.wallet.persist()?;
        self.save_last_scan_finished();

        self.notify_scan_complete().await?;

        Produces::ok(())
    }

    async fn update_sync_state_and_send_transactions(
        &mut self,
        scan_result: Result<SyncResponse, crate::node::client::Error>,
    ) -> ActorResult<()> {
        if scan_result.is_err() {
            self.state = ActorState::FailedSyncScan;
        }

        let scan_result: SyncResponse = scan_result?;
        self.wallet.bdk.apply_update(scan_result)?;
        self.wallet.persist()?;

        // get and send transactions
        let transactions = self.transactions().await?.await?;
        self.send(WalletManagerReconcileMessage::UpdatedTransactions(transactions));

        self.state = ActorState::SyncScanComplete;

        Produces::ok(())
    }

    /// Mark the wallet as scanned
    /// Notify the frontend that the wallet scan is complete
    /// Ssend the wallet balance and transaction
    async fn notify_scan_complete(&mut self) -> ActorResult<()> {
        use WalletManagerReconcileMessage as Msg;

        // reload the wallet from the file storage
        self.reload_wallet();

        // get and send wallet balance
        let balance = self.balance().await?.await.map_err_str(Error::WalletBalanceError)?;

        debug!("sending wallet balance: {balance:?}");
        self.send(Msg::WalletBalanceChanged(balance.into()));

        // get and send transactions
        let transactions: Vec<Transaction> =
            self.transactions().await?.await.map_err_str(Error::TransactionsRetrievalError)?;

        self.send(Msg::ScanComplete(transactions));

        Produces::ok(())
    }

    // reload the persisted wallet from the local file storage, for some reason
    // the balance is not updated after the second full scan if I don't reload
    // the wallet from the file storage
    fn reload_wallet(&mut self) {
        match Wallet::try_load_persisted(self.wallet.id.clone()) {
            Ok(wallet) => self.wallet = wallet,
            Err(error) => error!("failed to reload wallet: {error:?}"),
        }
    }

    fn last_scan_finished(&mut self) -> Option<Duration> {
        if let Some(last_scan_finished) = self.last_scan_finished {
            return Some(last_scan_finished);
        }

        let metadata = Database::global()
            .wallets()
            .get(&self.wallet.id, self.wallet.network, self.wallet.metadata.wallet_mode)
            .ok()??;

        let last_scan_finished = metadata.internal.last_scan_finished;
        self.last_scan_finished = last_scan_finished;

        last_scan_finished
    }

    fn save_last_scan_finished(&mut self) -> Option<()> {
        let now = UNIX_EPOCH.elapsed().unwrap();
        self.last_scan_finished = Some(now);

        let wallets = Database::global().wallets();

        let mut metadata = wallets
            .get(&self.wallet.id, self.wallet.network, self.wallet.metadata.wallet_mode)
            .ok()??;

        metadata.internal.last_scan_finished = Some(now);
        wallets.update_internal_metadata(&metadata).ok();
        self.wallet.metadata = metadata;

        Some(())
    }

    fn last_height_fetched(&mut self) -> Option<(Duration, usize)> {
        if let Some(last_height_fetched) = self.last_height_fetched {
            return Some(last_height_fetched);
        }

        let metadata = Database::global()
            .wallets()
            .get(&self.wallet.id, self.wallet.network, self.wallet.metadata.wallet_mode)
            .ok()??;

        let BlockSizeLast { block_height, last_seen } = &metadata.internal.last_height_fetched?;

        let last_height_fetched = Some((*last_seen, *(block_height) as usize));
        self.last_height_fetched = last_height_fetched;

        last_height_fetched
    }

    fn save_last_height_fetched(&mut self, block_height: usize) -> Option<()> {
        debug!("actor save_last_height_fetched");
        let now = UNIX_EPOCH.elapsed().unwrap();
        self.last_height_fetched = Some((now, block_height));

        let wallets = Database::global().wallets();
        let mut metadata = wallets
            .get(&self.wallet.id, self.wallet.network, self.wallet.metadata.wallet_mode)
            .ok()??;

        metadata.internal.last_height_fetched =
            Some(BlockSizeLast { block_height: block_height as u64, last_seen: now });

        wallets.update_internal_metadata(&metadata).ok();
        self.wallet.metadata = metadata.clone();

        Some(())
    }

    async fn node_client(&mut self) -> Result<&NodeClient, Error> {
        let node_client = self.node_client.as_ref();
        if node_client.is_none() {
            let node = Database::global().global_config.selected_node();
            let node_client = NodeClient::new(&node).await.map_err(|err| {
                Error::NodeConnectionFailed(format!("failed to create node client: {err}"))
            })?;

            self.node_client = Some(node_client);
        };

        Ok(self.node_client.as_ref().expect("just checked"))
    }
}

fn elapsed_secs_since(earlier: Duration) -> u64 {
    let now = UNIX_EPOCH.elapsed().expect("time went backwards");
    (now - earlier).as_secs()
}

impl WalletActor {
    fn send(&self, msg: WalletManagerReconcileMessage) {
        self.reconciler.send(msg.into()).unwrap();
    }
}

impl Drop for WalletActor {
    fn drop(&mut self) {
        debug!("[DROP] Wallet Actor for {}", self.wallet.id);
    }
}

async fn check_node_connection_inner(node: &Node) -> Result<(), String> {
    // Create a fresh client with its own TCP connection for this background check.
    // We cannot reuse the actor's cached client because:
    // 1. This runs in a background task (spawned via send_fut)
    // 2. The actor continues processing messages with its own client
    // 3. The underlying rust-electrum-client is NOT designed for concurrent access
    //    (it uses a "reader thread" pattern with try_lock that fails on concurrent use)
    // Creating a fresh connection ensures no shared state or concurrent access.
    //
    // TODO: We could optimize this to reuse the cached client when using esplora,
    // since esplora uses HTTP and doesn't have the concurrent access limitations
    // that electrum's persistent TCP connection has.
    let node_client = NodeClient::new(node)
        .await
        .map_err(|_| "unable to create a connection to the node".to_string())?;

    node_client
        .check_url()
        .with_timeout(Duration::from_secs(5))
        .await
        .map_err(|_| "unable to connect to node, timeout".to_string())?
        .map_err(|err| err.to_string())?;

    Ok(())
}
