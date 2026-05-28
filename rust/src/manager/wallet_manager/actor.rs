use crate::{
    database::{
        Database,
        wallet_data::{ReceiveAddressCache, WalletDataDb},
    },
    historical_price_service::HistoricalPriceService,
    manager::wallet_manager::{
        Error, SendFlowErrorAlert, WalletManagerError, WalletScanStatus,
        receive_address::{
            CACHE_WINDOW, ReceiveAddressPresentation, ReceiveAddressRefreshState,
            ReceiveAddressSession, ReceiveAddressState, ReceiveAddressStatus,
            RefreshExpiredAddressDecision,
        },
    },
    mnemonic,
    node::{
        Node,
        client::{Error as NodeError, NodeClient, NodeClientOptions},
        client_builder::NodeClientBuilder,
    },
    receive_address_watcher::ReceiveAddressWatcher,
    transaction::{ConfirmedTransaction, FeeRate, Transaction, TransactionDetails, TxId},
    transaction_watcher::TransactionWatcher,
    wallet::{
        Address, AddressInfo, Wallet, WalletAddressType, balance::Balance, metadata::BlockSizeLast,
    },
};
mod scan;

use act_zero::{runtimes::tokio::spawn_actor, *};
use act_zero_ext::into_actor_result;
use ahash::HashMap;
use bdk_wallet::{
    AddUtxoError, Utxo, WeightedUtxo,
    chain::{
        BlockId, TxGraph,
        bitcoin::Psbt,
        spk_client::{FullScanResponse, SyncRequest, SyncResponse},
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
use cove_bdk_progressive_scan::ScanUpdate;
use cove_common::consts::MIN_SEND_AMOUNT;
use cove_tokio::{AbortableTask, FutureTimeoutExt as _};
use cove_types::{
    confirm::{AddressAndAmount, ConfirmDetails, ExtraItem, InputOutputDetails, SplitOutput},
    fees::{FeeRateOptionWithTotalFee, FeeRateOptions, FeeRateOptionsWithTotalFee},
    utxo::{UtxoList, UtxoType},
};
use cove_util::result_ext::ResultExt as _;
use eyre::Result;
use flume::Sender;
use rand::RngExt as _;
use std::{
    sync::Arc,
    time::{Duration, UNIX_EPOCH},
};
use tap::TapFallible as _;
use tracing::{debug, error, info, warn};

use self::mnemonic::{Mnemonic, MnemonicExt as _};

use self::scan::{
    EMPTY_WALLET_SCAN_PROGRESS_DELAY, FullScanType, PreparedProgressiveScan,
    RETURNING_WALLET_SCAN_PROGRESS_DELAY, ScanProgressStart, ScanRequestOrder, WalletScanActor,
    WalletScanEvent, should_update_full_scan_metadata,
};
use super::{SingleOrMany, WalletManagerReconcileMessage};

const RECEIVE_ADDRESS_FRESHNESS_TIMEOUT: Duration = Duration::from_millis(400);

#[derive(Debug)]
pub struct WalletActor {
    pub addr: WeakAddr<Self>,
    pub reconciler: Sender<SingleOrMany>,
    pub wallet: Wallet,
    pub node_client: Option<NodeClient>,

    pub db: WalletDataDb,
    pub state: ActorState,
    pub receive_address: ReceiveAddressSession,

    seed: u64,
    transaction_watchers: HashMap<Txid, Addr<TransactionWatcher>>,
    receive_address_watcher: Option<Addr<ReceiveAddressWatcher>>,
    receive_address_refresh_timer: Option<AbortableTask<()>>,
    scan_actor: Option<Addr<WalletScanActor>>,

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

    FailedFullScan(FullScanType),
    FailedIncrementalScan,
    FailedSyncScan,
}

#[async_trait::async_trait]
impl Actor for WalletActor {
    async fn started(&mut self, addr: Addr<Self>) -> ActorResult<()> {
        self.addr = addr.downgrade();
        self.spawn_scan_actor();
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
        }

        false
    }
}

impl WalletActor {
    pub fn new(
        wallet: Wallet,
        reconciler: Sender<SingleOrMany>,
    ) -> Result<Self, crate::database::wallet_data::WalletDataError> {
        let db = WalletDataDb::new_or_existing(wallet.id.clone())?;
        let seed = rand::rng().random();

        Ok(Self {
            addr: Default::default(),
            reconciler,
            seed,
            wallet,
            node_client: None,
            last_scan_finished: None,
            last_height_fetched: None,
            state: ActorState::Initial,
            receive_address: ReceiveAddressSession::default(),
            transaction_watchers: HashMap::default(),
            receive_address_watcher: None,
            receive_address_refresh_timer: None,
            scan_actor: None,
            db,
        })
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
        self.wallet.unreserve_tx_change_addresses(&psbt.unsigned_tx);
        Ok(psbt)
    }

    // release reserved change addresses for a discarded unsigned transaction
    pub async fn cancel_txn(&mut self, txn: BdkTransaction) {
        self.wallet.unreserve_tx_change_addresses(&txn);
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
                }
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
        _payjoin_endpoint: Option<String>,
    ) -> ActorResult<Result<(), Error>> {
        // TODO: if payjoin_endpoint is Some, run BIP77 negotiation before broadcast
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

        self.do_broadcast_transaction(transaction.clone()).await?;

        // insert into local wallet and update UI immediately
        self.insert_broadcast_transaction(transaction).await;

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
            .broadcast_transaction(transaction.clone())
            .await
            .map_err(|error| {
                let error_string = format!("failed to broadcast transaction, try again: {error:?}");
                Error::SignAndBroadcastError(error_string)
            })?;

        // insert into local wallet and update UI immediately
        self.insert_broadcast_transaction(transaction).await;

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

    /// Insert a broadcast transaction into the local wallet and update the UI
    async fn insert_broadcast_transaction(&mut self, transaction: BdkTransaction) {
        use WalletManagerReconcileMessage as Msg;
        use std::time::SystemTime;

        let now =
            SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_else(|e| {
                warn!("System clock skew detected: {e}");
                u64::MAX
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
        match self.transactions().await {
            Ok(future) => match future.await {
                Ok(transactions) => self.send(Msg::UpdatedTransactions(transactions)),
                Err(e) => error!("Failed to get transactions after broadcast: {e}"),
            },
            Err(e) => error!("Failed to get transactions after broadcast: {e}"),
        }

        // start a transaction watcher to track confirmations
        send!(self.addr.start_transaction_watcher(txid));
    }

    pub async fn address_at(&mut self, index: u32) -> ActorResult<AddressInfo> {
        let address = self.wallet.bdk.peek_address(KeychainKind::External, index);
        Produces::ok(address.into())
    }

    pub async fn open_receive_address_intent(&mut self) -> ActorResult<()> {
        self.set_receive_address_loading(true);

        match self.do_open_receive_address().await {
            Ok(_) => self.set_receive_address_loading(false),
            Err(error) => {
                self.set_receive_address_loading(false);
                self.send(WalletManagerReconcileMessage::ReceiveAddressError(error.to_string()));
            }
        }

        Produces::ok(())
    }

    async fn do_open_receive_address(&mut self) -> Result<ReceiveAddressState, Error> {
        let now = current_epoch_secs();

        let Some(cache) = self.receive_address_cache()? else {
            let request_id = self.receive_address.next_request_id();
            return self.open_fresh_receive_address(request_id, now);
        };

        if !self.receive_address_cache_is_available(&cache, now) {
            let request_id = self.receive_address.next_request_id();
            return self.open_fresh_receive_address(request_id, now);
        }

        let derivation_index = cache.derivation_index;
        let request_id = self.receive_address.next_request_id();

        if !matches!(
            self.cached_receive_address_has_activity(derivation_index).await,
            Ok(Some(true))
        ) {
            return self.open_cached_receive_address(cache, request_id, now).await;
        }

        if let Err(error) = self.sync_receive_address_now(derivation_index).await {
            warn!("Failed to sync used receive address index={derivation_index}: {error}");
            self.wallet.mark_receive_address_used(derivation_index)?;
            self.notify_wallet_balance_and_transactions().await;

            self.start_targeted_receive_address_sync(request_id, derivation_index);
        }

        self.open_fresh_receive_address(request_id, now)
    }

    fn receive_address_cache_is_available(
        &self,
        cache: &ReceiveAddressCache,
        now_secs: u64,
    ) -> bool {
        let Some(expires_at_secs) = cache.first_shown_at_secs.checked_add(CACHE_WINDOW.as_secs())
        else {
            return false;
        };

        cache.address_type == self.wallet.metadata.address_type
            && cache.first_shown_at_secs <= now_secs
            && now_secs < expires_at_secs
            && self.wallet.receive_address_is_unused(cache.derivation_index)
    }

    fn receive_address_cache(&self) -> Result<Option<ReceiveAddressCache>, Error> {
        let cache = self.db.get_receive_address_cache().map_err_str(Error::ReceiveAddressError)?;

        Ok(cache.filter(|cache| {
            cache.wallet_id == self.wallet.id
                && cache.network == self.wallet.network
                && cache.address_type == self.wallet.metadata.address_type
        }))
    }

    fn open_fresh_receive_address(
        &mut self,
        request_id: u64,
        now: u64,
    ) -> Result<ReceiveAddressState, Error> {
        let state = self.new_receive_address_state(request_id, now, ReceiveAddressStatus::Fresh)?;

        Ok(state)
    }

    async fn open_cached_receive_address(
        &mut self,
        cache: ReceiveAddressCache,
        request_id: u64,
        now: u64,
    ) -> Result<ReceiveAddressState, Error> {
        let cache = cache.with_visible_window_start(now);
        let derivation_index = cache.derivation_index;
        self.db.set_receive_address_cache(cache).map_err_str(Error::ReceiveAddressError)?;

        let address = self.wallet.receive_address_at_index(derivation_index);
        let state = ReceiveAddressState::cached(
            request_id,
            address,
            ReceiveAddressStatus::CachedUnused,
            now,
        );
        self.receive_address.set_visible(state.clone());
        self.receive_address.set_refresh_state(ReceiveAddressRefreshState::Idle);
        self.send_receive_address_state(state.clone());

        self.start_receive_address_watcher(request_id, derivation_index);
        self.schedule_receive_address_refresh(&state, now);
        self.start_delayed_receive_address_activity_check(request_id, derivation_index);

        Ok(state)
    }

    pub async fn create_new_receive_address_intent(&mut self) -> ActorResult<()> {
        self.set_receive_address_loading(true);

        match self.do_create_new_receive_address() {
            Ok(_) => self.set_receive_address_loading(false),
            Err(error) => {
                self.set_receive_address_loading(false);
                self.send(WalletManagerReconcileMessage::ReceiveAddressError(error.to_string()));
            }
        }

        Produces::ok(())
    }

    fn do_create_new_receive_address(&mut self) -> Result<ReceiveAddressState, Error> {
        let request_id = self.receive_address.next_request_id();
        let now = current_epoch_secs();
        let state = self.new_receive_address_state(request_id, now, ReceiveAddressStatus::Fresh)?;

        Ok(state)
    }

    pub async fn refresh_expired_receive_address(&mut self, request_id: u64) -> ActorResult<()> {
        let now = current_epoch_secs();
        match self.receive_address.refresh_expired_decision(request_id, now) {
            RefreshExpiredAddressDecision::Rotate => {}
            RefreshExpiredAddressDecision::ReturnVisible(_)
            | RefreshExpiredAddressDecision::MissingVisibleState => return Produces::ok(()),
        }

        self.stop_receive_address_refresh_timer();
        self.set_receive_address_refresh_state(ReceiveAddressRefreshState::Refreshing);

        match self.new_receive_address_state(request_id, now, ReceiveAddressStatus::Fresh) {
            Ok(_) => Produces::ok(()),
            Err(error) => {
                if self.receive_address.visible_state().is_none() {
                    return Err(Error::ReceiveAddressError(
                        "receive request has no visible address".into(),
                    )
                    .into());
                }

                warn!("Failed to refresh receive address request_id={request_id}: {error}");
                self.set_receive_address_refresh_state(ReceiveAddressRefreshState::Failed);
                Produces::ok(())
            }
        }
    }

    pub async fn close_receive_address(&mut self, request_id: u64) {
        if self.receive_address.close(request_id) {
            self.stop_receive_address_watcher();
            self.stop_receive_address_refresh_timer();
            self.set_receive_address_loading(false);
            self.send_receive_address_presentation();
        }

        self.send(WalletManagerReconcileMessage::ReceiveAddressClosed(request_id));
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

        let scan_progress_start = {
            let initial_balance =
                self.balance().await?.await.map_err_str(Error::WalletBalanceError)?;

            self.send(Msg::WalletBalanceChanged(initial_balance.into()));

            let initial_transactions =
                self.transactions().await?.await.map_err_str(Error::TransactionsRetrievalError)?;

            let progress_start = wallet_scan_progress_start(
                self.wallet.metadata.internal.performed_full_scan_at.is_some(),
                initial_transactions.is_empty(),
            );

            self.send(Msg::AvailableTransactions(initial_transactions));

            progress_start
        };

        // start the wallet scan in a background task
        self.start_wallet_scan_in_task(force_scan, scan_progress_start)
            .await?
            .await
            .map_err_str(Error::WalletScanError)?;

        Produces::ok(())
    }

    pub async fn start_wallet_scan_in_task(
        &mut self,
        force_scan: bool,
        progress_start: ScanProgressStart,
    ) -> ActorResult<()> {
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
            send!(addr.perform_incremental_scan(progress_start));
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
        self.addr.send_fut_with(|addr| async move {
            tokio::time::sleep(Duration::from_secs(30)).await;
            send!(addr.perform_scan_for_single_tx_id(tx_id));
            send!(addr.remove_watcher_for_txn(tx_id));
        });

        Produces::ok(())
    }

    pub async fn shutdown(&mut self) {
        debug!("shutdown wallet actor");
        if let Some(scan_actor) = &self.scan_actor {
            send!(scan_actor.shutdown());
        }

        self.stop_receive_address_watcher();
        self.stop_receive_address_refresh_timer();
        self.transaction_watchers = HashMap::default();
        self.send_scan_status(WalletScanStatus::Idle);
    }

    async fn remove_watcher_for_txn(&mut self, tx_id: Txid) {
        debug!("removing watcher for txn: {tx_id}");
        self.transaction_watchers.remove(&tx_id);
    }

    pub async fn perform_scan_for_single_tx_id(&mut self, tx_id: Txid) -> ActorResult<()> {
        let start = UNIX_EPOCH.elapsed().unwrap().as_secs();
        let _ = self.update_height().await?.await;

        let chain_tip = self.wallet.bdk.local_chain().tip();
        let sync_request_builder = SyncRequest::builder().txids(vec![tx_id]).chain_tip(chain_tip);

        let sync_request = sync_request_builder.build();

        let node_client = self.node_client().await?.clone();
        let graph = self.wallet.bdk.tx_graph().clone();

        let now = UNIX_EPOCH.elapsed().unwrap().as_secs();
        debug!("done scan for spk in {}s", now - start);
        self.addr.send_fut_with(|addr| async move {
            let scan_result = node_client.sync(&graph, sync_request).await;

            let now = UNIX_EPOCH.elapsed().unwrap().as_secs();
            debug!("done single txn id sync scan in {}s", now - start);

            // save updated txns and send to frontend
            send!(addr.update_sync_state_and_send_transactions(scan_result));
        });

        Produces::ok(())
    }

    async fn perform_full_scan(&mut self) -> ActorResult<()> {
        if self.state != ActorState::Initial {
            debug!("already performing scanning or scanned skipping ({:?})", self.state);

            return Produces::ok(());
        }

        debug!("starting full scan");
        let scan_actor = self.scan_actor();
        send!(scan_actor.start_full_scan(ScanProgressStart::Immediate));

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
        }

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
            self.wallet.unreserve_tx_change_addresses(&fee_psbt.unsigned_tx);
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

    /// Perform a full scan with a user-supplied gap limit to recover missed addresses.
    pub async fn perform_rescan_full_scan(&mut self, gap_limit: u32) -> ActorResult<()> {
        debug!("perform_rescan_full_scan with gap_limit={gap_limit}");

        let scan_actor = self.scan_actor();
        send!(scan_actor.start_rescan(gap_limit));

        Produces::ok(())
    }

    async fn prepare_progressive_scan(
        &mut self,
        request_order: ScanRequestOrder,
    ) -> ActorResult<PreparedProgressiveScan> {
        let node_client = self.node_client().await?.clone();

        let full_scan_request = match request_order {
            ScanRequestOrder::Standard => self.wallet.bdk.start_full_scan().build(),
            ScanRequestOrder::ReceivePriority => self.wallet.start_receive_prioritized_full_scan(),
        };
        let graph = self.wallet.bdk.tx_graph().clone();
        let last_revealed_indices = self.wallet.bdk.spk_index().last_revealed_indices();

        Produces::ok(PreparedProgressiveScan {
            node_client,
            graph,
            full_scan_request,
            last_revealed_indices,
        })
    }

    async fn perform_incremental_scan(
        &mut self,
        progress_start: ScanProgressStart,
    ) -> ActorResult<()> {
        debug!("starting incremental scan");

        let scan_actor = self.scan_actor();
        send!(scan_actor.start_incremental_scan(progress_start));

        Produces::ok(())
    }

    async fn handle_wallet_scan_event(&mut self, event: WalletScanEvent) -> ActorResult<()> {
        match event {
            WalletScanEvent::FullScanStarted(scan_type) => {
                self.state = ActorState::PerformingFullScan(scan_type);
            }
            WalletScanEvent::IncrementalScanStarted => {
                self.state = ActorState::PerformingIncrementalScan;
            }
            WalletScanEvent::FullScanPrepareFailed(scan_type) => {
                self.state = state_after_full_scan_prepare_failed(
                    scan_type,
                    self.wallet.metadata.internal.performed_full_scan_at.is_some(),
                );
            }
            WalletScanEvent::IncrementalScanPrepareFailed => {
                self.state = ActorState::FailedIncrementalScan;
            }
            WalletScanEvent::StatusChanged(status) => {
                self.send_scan_status(status);
            }
            WalletScanEvent::PartialUpdate(scan_update) => {
                self.handle_progressive_scan_update(scan_update);
            }
            WalletScanEvent::FlushUi => {
                self.flush_progressive_scan_ui().await;
            }
            WalletScanEvent::FullScanFinished { scan_type, result } => {
                self.handle_full_scan_complete(result, scan_type).await?;
            }
            WalletScanEvent::IncrementalScanFinished { result } => {
                self.handle_incremental_scan_complete(result).await?;
            }
        }

        Produces::ok(())
    }

    fn handle_progressive_scan_update(&mut self, scan_update: ScanUpdate<KeychainKind>) {
        if let Err(error) = self.apply_progressive_scan_update(scan_update) {
            error!("Failed to apply progressive scan update: {error}");
            self.send(WalletManagerReconcileMessage::WalletError(Error::WalletScanError(format!(
                "failed to apply progressive scan update: {error}"
            ))));
        }
    }

    fn apply_progressive_scan_update(
        &mut self,
        scan_update: ScanUpdate<KeychainKind>,
    ) -> Result<()> {
        if scan_update.is_empty() {
            return Ok(());
        }

        self.wallet.bdk.apply_update(progressive_scan_update_response(scan_update))?;
        self.wallet.persist()?;

        Ok(())
    }

    async fn flush_progressive_scan_ui(&mut self) {
        let balance = self.wallet.balance();
        self.send(WalletManagerReconcileMessage::WalletBalanceChanged(balance.into()));

        let transactions = self.do_transactions().await;
        self.send(WalletManagerReconcileMessage::UpdatedTransactions(transactions));
    }

    async fn handle_full_scan_complete(
        &mut self,
        full_scan_result: Result<FullScanResponse<KeychainKind>, NodeError>,
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
                self.send_scan_status(WalletScanStatus::Idle);
                return Err(error.into());
            }
        }

        if should_update_full_scan_metadata(full_scan_type) {
            let now = jiff::Timestamp::now().as_second() as u64;
            self.wallet.metadata.internal.performed_full_scan_at = Some(now);
            Database::global().wallets.update_internal_metadata(&self.wallet.metadata)?;
        }

        self.save_last_scan_finished();
        self.notify_scan_complete().await?;

        self.state = ActorState::FullScanComplete(full_scan_type);
        self.send_scan_status(WalletScanStatus::Idle);

        Produces::ok(())
    }

    async fn handle_incremental_scan_complete(
        &mut self,
        scan_result: Result<FullScanResponse<KeychainKind>, NodeError>,
    ) -> ActorResult<()> {
        let sync_result = match scan_result {
            Ok(sync_result) => sync_result,
            Err(error) => {
                self.state = ActorState::FailedIncrementalScan;
                self.send_scan_status(WalletScanStatus::Idle);
                return Err(error.into());
            }
        };

        self.wallet.bdk.apply_update(sync_result)?;
        self.wallet.persist()?;
        self.save_last_scan_finished();

        self.notify_scan_complete().await?;
        self.send_scan_status(WalletScanStatus::Idle);

        Produces::ok(())
    }

    async fn update_sync_state_and_send_transactions(
        &mut self,
        scan_result: Result<SyncResponse, NodeError>,
    ) -> ActorResult<()> {
        if scan_result.is_err() {
            self.state = ActorState::FailedSyncScan;
        }

        let scan_result: SyncResponse = scan_result?;
        self.wallet.bdk.apply_update(scan_result)?;
        self.wallet.persist()?;
        self.update_visible_receive_address_payment_status(None);

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
        self.update_visible_receive_address_payment_status(None);

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
        let now = UNIX_EPOCH.elapsed().unwrap_or_default();
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
        let now = UNIX_EPOCH.elapsed().unwrap_or_default();
        self.last_height_fetched = Some((now, block_height));

        let wallets = Database::global().wallets();
        let mut metadata = wallets
            .get(&self.wallet.id, self.wallet.network, self.wallet.metadata.wallet_mode)
            .ok()??;

        let last_height_fetched =
            BlockSizeLast { block_height: block_height as u64, last_seen: now };

        metadata.internal.last_height_fetched = Some(last_height_fetched);
        wallets.update_internal_metadata(&metadata).ok();

        Database::global()
            .global_cache
            .set_block_height(self.wallet.network, last_height_fetched)
            .ok();

        self.wallet.metadata = metadata.clone();

        Some(())
    }

    fn new_receive_address_state(
        &mut self,
        request_id: u64,
        now: u64,
        status: ReceiveAddressStatus,
    ) -> Result<ReceiveAddressState, Error> {
        let address = self.wallet.get_next_address()?;
        let cache = ReceiveAddressCache {
            derivation_index: address.info.index,
            first_shown_at_secs: now,
            wallet_id: self.wallet.id.clone(),
            network: self.wallet.network,
            address_type: self.wallet.metadata.address_type,
        };

        self.db.set_receive_address_cache(cache).map_err_str(Error::ReceiveAddressError)?;

        let state = ReceiveAddressState::cached(request_id, address, status, now);
        self.receive_address.set_visible(state.clone());
        self.receive_address.set_refresh_state(ReceiveAddressRefreshState::Idle);
        self.send_receive_address_state(state.clone());

        if status == ReceiveAddressStatus::Fresh {
            self.start_receive_address_watcher(request_id, state.address.info.index);
        } else {
            self.stop_receive_address_watcher();
        }

        self.schedule_receive_address_refresh(&state, now);

        Ok(state)
    }

    fn set_receive_address_loading(&self, is_loading: bool) {
        self.send(WalletManagerReconcileMessage::ReceiveAddressLoadingChanged(is_loading));
    }

    fn set_receive_address_refresh_state(&mut self, refresh_state: ReceiveAddressRefreshState) {
        self.receive_address.set_refresh_state(refresh_state);
        self.send_receive_address_presentation();
    }

    fn send_receive_address_state(&self, state: ReceiveAddressState) {
        self.send(WalletManagerReconcileMessage::ReceiveAddressUpdated(state));
        self.send_receive_address_presentation();
    }

    fn send_receive_address_presentation(&self) {
        let presentation: ReceiveAddressPresentation = self.receive_address.presentation();
        self.send(WalletManagerReconcileMessage::ReceiveAddressPresentationUpdated(presentation));
    }

    fn schedule_receive_address_refresh(&mut self, state: &ReceiveAddressState, now_secs: u64) {
        self.stop_receive_address_refresh_timer();

        let Some(delay) = state.refresh_delay(now_secs) else {
            return;
        };

        let request_id = state.request_id;
        let addr = self.addr.clone();
        self.receive_address_refresh_timer = Some(AbortableTask::spawn(async move {
            tokio::time::sleep(delay).await;
            send!(addr.refresh_expired_receive_address(request_id));
        }));
    }

    fn stop_receive_address_refresh_timer(&mut self) {
        self.receive_address_refresh_timer = None;
    }

    async fn cached_receive_address_has_activity(
        &mut self,
        derivation_index: u32,
    ) -> Result<Option<bool>, Error> {
        let node_client = self.node_client().await?.clone();
        let address =
            self.wallet.bdk.peek_address(KeychainKind::External, derivation_index).address;

        match node_client
            .check_address_for_txn(address)
            .with_timeout(RECEIVE_ADDRESS_FRESHNESS_TIMEOUT)
            .await
        {
            Ok(Ok(has_activity)) => Ok(Some(has_activity)),
            Ok(Err(error)) => Err(Error::NodeConnectionFailed(error.to_string())),
            Err(_) => Ok(None),
        }
    }

    fn start_delayed_receive_address_activity_check(
        &mut self,
        request_id: u64,
        derivation_index: u32,
    ) {
        let node = Database::global().global_config.selected_node();
        let address =
            self.wallet.bdk.peek_address(KeychainKind::External, derivation_index).address;
        self.addr.send_fut_with(|addr| async move {
            let result = match NodeClient::new(&node).await {
                Ok(node_client) => node_client.check_address_for_txn(address).await,
                Err(error) => Err(error),
            };

            send!(addr.handle_receive_address_activity_result(
                request_id,
                derivation_index,
                result
            ));
        });
    }

    async fn handle_receive_address_activity_result(
        &mut self,
        request_id: u64,
        derivation_index: u32,
        result: Result<bool, NodeError>,
    ) -> ActorResult<()> {
        if !self.receive_address.is_current(request_id) {
            return Produces::ok(());
        }

        if result.unwrap_or(false) {
            self.start_targeted_receive_address_sync(request_id, derivation_index);
        }

        Produces::ok(())
    }

    pub async fn handle_receive_address_watch_activity(
        &mut self,
        request_id: u64,
        derivation_index: u32,
    ) -> ActorResult<()> {
        if !self.receive_address.can_mark_payment_received(request_id, derivation_index) {
            return Produces::ok(());
        }

        self.stop_receive_address_watcher();
        self.db.delete_receive_address_cache().map_err_str(Error::ReceiveAddressError)?;

        if self.wallet.receive_address_is_unused(derivation_index) {
            self.wallet.mark_receive_address_used(derivation_index)?;
        }

        let Some(state) = self.receive_address.mark_payment_received(request_id, derivation_index)
        else {
            return Produces::ok(());
        };

        self.stop_receive_address_refresh_timer();
        self.receive_address.set_refresh_state(ReceiveAddressRefreshState::Idle);
        self.send_receive_address_state(state);
        self.start_targeted_receive_address_sync(request_id, derivation_index);

        Produces::ok(())
    }

    fn start_receive_address_watcher(&mut self, request_id: u64, derivation_index: u32) {
        self.stop_receive_address_watcher();

        let node = Database::global().global_config.selected_node();
        let options = NodeClientOptions { batch_size: 1 };
        let client_builder = NodeClientBuilder { node, options };

        let address =
            self.wallet.bdk.peek_address(KeychainKind::External, derivation_index).address;

        let watcher = ReceiveAddressWatcher::new(
            self.addr.clone(),
            request_id,
            derivation_index,
            address,
            client_builder,
            CACHE_WINDOW,
        );

        self.receive_address_watcher = Some(spawn_actor(watcher));
    }

    fn stop_receive_address_watcher(&mut self) {
        if let Some(watcher) = self.receive_address_watcher.take() {
            send!(watcher.stop_watching());
        }
    }

    pub async fn handle_receive_address_watcher_stopped(
        &mut self,
        request_id: u64,
        derivation_index: u32,
    ) -> ActorResult<()> {
        if self.receive_address.is_current(request_id)
            && self.receive_address.visible_state().is_some_and(|state| {
                state.status == ReceiveAddressStatus::PaymentReceived
                    || state.address.info.index == derivation_index
            })
        {
            self.receive_address_watcher = None;
        }

        Produces::ok(())
    }

    fn start_targeted_receive_address_sync(&mut self, request_id: u64, derivation_index: u32) {
        let (node, graph, sync_request) = self.receive_address_sync_inputs(derivation_index);
        self.addr.send_fut_with(|addr| async move {
            let result = match NodeClient::new(&node).await {
                Ok(node_client) => node_client.sync(&graph, sync_request).await,
                Err(error) => Err(error),
            };

            send!(addr.handle_receive_address_sync_result(request_id, derivation_index, result));
        });
    }

    async fn sync_receive_address_now(&mut self, derivation_index: u32) -> Result<(), Error> {
        let (node, graph, sync_request) = self.receive_address_sync_inputs(derivation_index);
        let node_client = NodeClient::new(&node)
            .await
            .map_err_prefix("failed to create node client", Error::NodeConnectionFailed)?;

        let sync_result =
            node_client.sync(&graph, sync_request).await.map_err_str(Error::ReceiveAddressError)?;
        self.wallet.bdk.apply_update(sync_result).map_err_str(Error::ReceiveAddressError)?;

        if self.wallet.receive_address_is_unused(derivation_index) {
            self.wallet.mark_receive_address_used(derivation_index)?;
        } else {
            self.wallet.persist()?;
        }

        self.notify_wallet_balance_and_transactions().await;

        Ok(())
    }

    fn receive_address_sync_inputs(
        &mut self,
        derivation_index: u32,
    ) -> (Node, TxGraph, SyncRequest<(KeychainKind, u32)>) {
        let node = Database::global().global_config.selected_node();
        let address =
            self.wallet.bdk.peek_address(KeychainKind::External, derivation_index).address;
        let script_pubkey = address.script_pubkey();
        let chain_tip = self.wallet.bdk.local_chain().tip();

        let sync_request = SyncRequest::builder()
            .chain_tip(chain_tip)
            .spks_with_indexes([((KeychainKind::External, derivation_index), script_pubkey)])
            .build();

        let graph = self.wallet.bdk.tx_graph().clone();

        (node, graph, sync_request)
    }

    async fn handle_receive_address_sync_result(
        &mut self,
        request_id: u64,
        derivation_index: u32,
        result: Result<SyncResponse, NodeError>,
    ) -> ActorResult<()> {
        let sync_result = result
            .inspect_err(|error| warn!("Address sync failed index={derivation_index}: {error}"))
            .map_err_str(Error::ReceiveAddressError)?;

        self.wallet.bdk.apply_update(sync_result)?;

        if !self.receive_address.is_current(request_id) {
            self.wallet.persist()?;
            self.notify_wallet_balance_and_transactions().await;

            return Produces::ok(());
        }

        if let Some(state) = self.receive_address.visible_state()
            && state.address.info.index == derivation_index
            && state.status != ReceiveAddressStatus::PaymentReceived
            && self.wallet.receive_address_is_unused(state.address.info.index)
        {
            self.wallet.mark_receive_address_used(state.address.info.index)?;
        }

        self.wallet.persist()?;

        self.update_visible_receive_address_payment_status(Some(derivation_index));
        self.notify_wallet_balance_and_transactions().await;

        Produces::ok(())
    }

    async fn notify_wallet_balance_and_transactions(&mut self) {
        let balance = self.wallet.balance();
        self.send(WalletManagerReconcileMessage::WalletBalanceChanged(balance.into()));

        let transactions = self.do_transactions().await;
        self.send(WalletManagerReconcileMessage::UpdatedTransactions(transactions));
    }

    fn update_visible_receive_address_payment_status(
        &mut self,
        derivation_index: Option<u32>,
    ) -> bool {
        let Some(state) = self.receive_address.visible_state() else {
            return false;
        };

        if state.status == ReceiveAddressStatus::PaymentReceived
            || derivation_index.is_some_and(|index| state.address.info.index != index)
            || self.wallet.receive_address_is_unused(state.address.info.index)
        {
            return false;
        }

        let state = state.payment_received();
        self.receive_address.set_visible(state.clone());
        self.receive_address.set_refresh_state(ReceiveAddressRefreshState::Idle);
        self.stop_receive_address_watcher();
        self.stop_receive_address_refresh_timer();
        self.send_receive_address_state(state);

        true
    }

    async fn node_client(&mut self) -> Result<&NodeClient, Error> {
        let node_client = self.node_client.as_ref();
        if node_client.is_none() {
            let node = Database::global().global_config.selected_node();
            let node_client = NodeClient::new(&node)
                .await
                .map_err_prefix("failed to create node client", Error::NodeConnectionFailed)?;

            self.node_client = Some(node_client);
        }

        Ok(self.node_client.as_ref().expect("just checked"))
    }
}

fn elapsed_secs_since(earlier: Duration) -> u64 {
    let now = UNIX_EPOCH.elapsed().unwrap_or(earlier);
    now.saturating_sub(earlier).as_secs()
}

fn current_epoch_secs() -> u64 {
    UNIX_EPOCH.elapsed().unwrap_or_default().as_secs()
}

fn progressive_scan_update_response(
    scan_update: ScanUpdate<KeychainKind>,
) -> FullScanResponse<KeychainKind> {
    FullScanResponse {
        chain_update: scan_update.chain_update,
        tx_update: scan_update.tx_update,
        last_active_indices: scan_update.last_active_indices,
    }
}

fn state_after_full_scan_prepare_failed(
    scan_type: FullScanType,
    completed_full_scan: bool,
) -> ActorState {
    if !completed_full_scan {
        return ActorState::Initial;
    }

    ActorState::FailedFullScan(scan_type)
}

fn wallet_scan_progress_start(
    completed_full_scan: bool,
    cached_transactions_empty: bool,
) -> ScanProgressStart {
    if !completed_full_scan {
        return ScanProgressStart::Immediate;
    }

    if cached_transactions_empty {
        return ScanProgressStart::Delayed(EMPTY_WALLET_SCAN_PROGRESS_DELAY);
    }

    ScanProgressStart::Delayed(RETURNING_WALLET_SCAN_PROGRESS_DELAY)
}

impl WalletActor {
    fn send(&self, msg: WalletManagerReconcileMessage) {
        self.reconciler.send(msg.into()).unwrap();
    }

    fn send_scan_status(&self, status: WalletScanStatus) {
        self.send(WalletManagerReconcileMessage::WalletScanStatusChanged(status));
    }

    fn scan_actor(&mut self) -> Addr<WalletScanActor> {
        if let Some(scan_actor) = &self.scan_actor {
            return scan_actor.clone();
        }

        self.spawn_scan_actor()
    }

    fn spawn_scan_actor(&mut self) -> Addr<WalletScanActor> {
        let scan_actor = spawn_actor(WalletScanActor::new(self.addr.clone()));
        self.watch_scan_actor_termination(scan_actor.clone());
        self.scan_actor = Some(scan_actor.clone());
        scan_actor
    }

    fn watch_scan_actor_termination(&self, scan_actor: Addr<WalletScanActor>) {
        let addr = self.addr.clone();
        self.addr.send_fut(async move {
            scan_actor.termination().await;
            send!(addr.clear_scan_actor_if_stopped(scan_actor));
        });
    }

    async fn clear_scan_actor_if_stopped(&mut self, stopped_scan_actor: Addr<WalletScanActor>) {
        if self.scan_actor.as_ref().is_some_and(|scan_actor| scan_actor == &stopped_scan_actor) {
            self.scan_actor = None;
        }
    }
}

impl Drop for WalletActor {
    fn drop(&mut self) {
        let _ = self.reconciler.send(
            WalletManagerReconcileMessage::WalletScanStatusChanged(WalletScanStatus::Idle).into(),
        );

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

#[cfg(test)]
mod tests {
    use bdk_wallet::KeychainKind;
    use cove_bdk_progressive_scan::ScanUpdate;
    use std::collections::BTreeMap;

    use super::{
        ActorState, EMPTY_WALLET_SCAN_PROGRESS_DELAY, FullScanType,
        RETURNING_WALLET_SCAN_PROGRESS_DELAY, ScanProgressStart, progressive_scan_update_response,
        wallet_scan_progress_start,
    };

    #[test]
    fn progressive_scan_update_response_preserves_last_active_indices() {
        let scan_update = ScanUpdate {
            chain_update: None,
            tx_update: Default::default(),
            last_active_indices: BTreeMap::from([(KeychainKind::External, 7)]),
        };

        let response = progressive_scan_update_response(scan_update);

        assert_eq!(response.last_active_indices, BTreeMap::from([(KeychainKind::External, 7)]));
    }

    #[test]
    fn prepare_failure_before_first_full_scan_returns_to_initial_state() {
        assert_eq!(
            super::state_after_full_scan_prepare_failed(FullScanType::Full, false),
            ActorState::Initial
        );
    }

    #[test]
    fn prepare_failure_after_completed_full_scan_records_failed_scan() {
        assert_eq!(
            super::state_after_full_scan_prepare_failed(FullScanType::Rescan(50), true),
            ActorState::FailedFullScan(FullScanType::Rescan(50))
        );
    }

    #[test]
    fn first_full_scan_uses_immediate_progress() {
        assert_eq!(wallet_scan_progress_start(false, true), ScanProgressStart::Immediate);
        assert_eq!(wallet_scan_progress_start(false, false), ScanProgressStart::Immediate);
    }

    #[test]
    fn returning_wallet_with_transactions_delays_progress() {
        assert_eq!(
            wallet_scan_progress_start(true, false),
            ScanProgressStart::Delayed(RETURNING_WALLET_SCAN_PROGRESS_DELAY)
        );
    }

    #[test]
    fn empty_returning_wallet_delays_progress_separately() {
        assert_eq!(
            wallet_scan_progress_start(true, true),
            ScanProgressStart::Delayed(EMPTY_WALLET_SCAN_PROGRESS_DELAY)
        );
    }
}
