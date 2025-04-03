use crate::{
    consts::GAP_LIMIT,
    database::{Database, wallet_data::WalletDataDb},
    manager::wallet::{Error, SendFlowErrorAlert, WalletManagerError},
    mnemonic,
    node::client::NodeClient,
    transaction::{FeeRate, Transaction, TransactionDetails, TxId, fees::BdkFeeRate},
    wallet::{
        Address, AddressInfo, Wallet, WalletAddressType,
        balance::Balance,
        confirm::{AddressAndAmount, ConfirmDetails, InputOutputDetails, SplitOutput},
        metadata::BlockSizeLast,
    },
};
use act_zero::*;
use bdk_chain::{TxGraph, bitcoin::Psbt, spk_client::FullScanResponse};
use bdk_core::spk_client::FullScanRequest;
use bdk_wallet::{KeychainKind, SignOptions, TxOrdering};
use bitcoin::Amount;
use bitcoin::{Transaction as BdkTransaction, params::Params};
use crossbeam::channel::Sender;
use eyre::Result;
use std::time::{Duration, UNIX_EPOCH};
use tap::TapFallible as _;
use tracing::{debug, error};

use self::mnemonic::{Mnemonic, MnemonicExt as _};

use super::WalletManagerReconcileMessage;

#[derive(Debug)]
pub struct WalletActor {
    pub addr: WeakAddr<Self>,
    pub reconciler: Sender<WalletManagerReconcileMessage>,
    pub wallet: Wallet,
    pub node_client: Option<NodeClient>,

    pub db: WalletDataDb,
    pub state: ActorState,

    // cached values, source of truth is the redb database saved with wallet metadata
    last_scan_finished: Option<Duration>,
    last_height_fetched: Option<(Duration, usize)>,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum ActorState {
    Initial,
    PerformingIncrementalScan,
    PerformingFullScan(FullScanType),

    FullScanComplete(FullScanType),

    /// incremental scan is performed after the expanded full scan
    IncrementalScanComplete,

    FailedFullScan(FullScanType),
    FailedIncrementalScan,
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
                self.send(WalletManagerReconcileMessage::NodeConnectionFailed(
                    error_string,
                ));
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
    pub fn new(wallet: Wallet, reconciler: Sender<WalletManagerReconcileMessage>) -> Self {
        let db = WalletDataDb::new_or_existing(wallet.id.clone());

        Self {
            addr: Default::default(),
            reconciler,
            wallet,
            node_client: None,
            last_scan_finished: None,
            last_height_fetched: None,
            state: ActorState::Initial,
            db,
        }
    }

    pub async fn balance(&mut self) -> ActorResult<Balance> {
        let balance = self.wallet.balance();
        Produces::ok(balance)
    }

    // Build a transaction but don't advance the change address index
    pub async fn build_ephemeral_drain_tx(
        &mut self,
        address: Address,
        fee: FeeRate,
    ) -> ActorResult<Psbt> {
        let script_pubkey = address.script_pubkey();
        let mut tx_builder = self.wallet.bdk.build_tx();

        tx_builder
            .drain_wallet()
            .drain_to(script_pubkey)
            .fee_rate(fee.into());

        let psbt = tx_builder.finish()?;
        self.wallet.bdk.cancel_tx(&psbt.unsigned_tx);

        Produces::ok(psbt)
    }

    pub async fn build_tx(
        &mut self,
        amount: Amount,
        address: Address,
        fee_rate: BdkFeeRate,
    ) -> ActorResult<Result<Psbt, Error>> {
        let psbt = self.do_build_tx(amount, address, fee_rate).await;
        Produces::ok(psbt)
    }

    async fn do_build_tx(
        &mut self,
        amount: Amount,
        address: Address,
        fee_rate: BdkFeeRate,
    ) -> Result<Psbt, Error> {
        let script_pubkey = address.script_pubkey();

        let mut tx_builder = self.wallet.bdk.build_tx();
        tx_builder.ordering(TxOrdering::Untouched);
        tx_builder.add_recipient(script_pubkey, amount);
        tx_builder.fee_rate(fee_rate);

        let psbt = tx_builder
            .finish()
            .map_err(|err| Error::BuildTxError(err.to_string()))?;

        Ok(psbt)
    }

    // Build a transaction but don't advance the change address index
    pub async fn build_ephemeral_tx(
        &mut self,
        amount: Amount,
        address: Address,
        fee: BdkFeeRate,
    ) -> ActorResult<Psbt> {
        let psbt = self.do_build_tx(amount, address, fee).await?;
        self.wallet.bdk.cancel_tx(&psbt.unsigned_tx);
        Produces::ok(psbt)
    }

    // cancel a transaction, reset the address & change address index
    pub async fn cancel_txn(&mut self, txn: BdkTransaction) {
        self.wallet.bdk.cancel_tx(&txn)
    }

    pub async fn transactions(&mut self) -> ActorResult<Vec<Transaction>> {
        let zero = Amount::ZERO.into();

        let mut transactions = self.wallet.bdk
            .transactions()
            .map(|tx| {
                let sent_and_received = self.wallet.bdk.sent_and_received(&tx.tx_node.tx).into();
                (tx, sent_and_received)
            })
            .map(|(tx, sent_and_received)| Transaction::new(&self.wallet.id, sent_and_received, tx))
            .filter(|tx| tx.sent_and_received().amount() > zero)
            .collect::<Vec<Transaction>>();

        transactions.sort_unstable_by(|a, b| a.cmp(b).reverse());

        Produces::ok(transactions)
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

    pub async fn get_confirm_details(
        &mut self,
        psbt: Psbt,
        fee_rate: BdkFeeRate,
    ) -> ActorResult<Result<ConfirmDetails, Error>> {
        let details = self.do_get_confirm_details(psbt, fee_rate).await;
        Produces::ok(details)
    }

    async fn do_get_confirm_details(
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
            return Err(error(
                "multiple address to send to found, not currently supported",
            ));
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
        let fee = psbt
            .fee()
            .map_err(|err| err_fmt("unable to get fee", err))?;

        let spending_amount = sending_amount
            .checked_add(fee)
            .ok_or_else(|| error("fee overflow, cannot calculate spending amount"))?;

        let psbt = psbt.into();
        let more_details = InputOutputDetails::new(&psbt, network);
        let details = ConfirmDetails {
            spending_amount: spending_amount.into(),
            sending_amount: sending_amount.into(),
            fee_total: fee.into(),
            fee_rate: fee_rate.into(),
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

    pub async fn broadcast_transaction(
        &mut self,
        transaction: BdkTransaction,
    ) -> ActorResult<Result<(), Error>> {
        let result = self.do_broadcast_transaction(transaction).await;
        Produces::ok(result)
    }

    async fn do_broadcast_transaction(&mut self, transaction: BdkTransaction) -> Result<(), Error> {
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

    pub async fn finalize_psbt(
        &mut self,
        psbt: Psbt,
    ) -> ActorResult<Result<bitcoin::Transaction, Error>> {
        let tx = self.do_finalize_psbt(psbt).await;
        Produces::ok(tx)
    }

    pub async fn do_finalize_psbt(&mut self, psbt: Psbt) -> Result<bitcoin::Transaction, Error> {
        let mut psbt = psbt;

        let finalized = self
            .wallet
            .bdk
            .finalize_psbt(&mut psbt, SignOptions::default())
            .map_err(|e| Error::PsbtFinalizeError(e.to_string()))?;

        if !finalized {
            return Err(Error::PsbtFinalizeError(
                "Failed to finalize PSBT".to_string(),
            ));
        }

        let tx = psbt.extract_tx().map_err(|e| {
            let err = format!("Failed to extract tx from PSBT: {e:?}");
            Error::PsbtFinalizeError(err)
        })?;

        Ok(tx)
    }

    pub async fn address_at(&mut self, index: u32) -> ActorResult<AddressInfo> {
        let address = self
            .wallet
            .bdk
            .peek_address(KeychainKind::External, index);
        Produces::ok(address.into())
    }

    pub async fn next_address(&mut self) -> ActorResult<AddressInfo> {
        let address = self.wallet.get_next_address()?;
        Produces::ok(address)
    }

    pub async fn check_node_connection(&mut self) -> ActorResult<()> {
        self.node_client()
            .await?
            .check_url()
            .await
            .map_err(|error| Error::NodeConnectionFailed(error.to_string()))?;

        Produces::ok(())
    }

    pub async fn wallet_scan_and_notify(&mut self, force_scan: bool) -> ActorResult<()> {
        use WalletManagerReconcileMessage as Msg;
        debug!("wallet_scan_and_notify");

        // get the initial balance and transactions
        {
            let initial_balance = self
                .balance()
                .await?
                .await
                .map_err(|error| Error::WalletBalanceError(error.to_string()))?;

            self.send(Msg::WalletBalanceChanged(initial_balance.into()));

            let initial_transactions = self
                .transactions()
                .await?
                .await
                .map_err(|error| Error::TransactionsRetrievalError(error.to_string()))?;

            self.send(Msg::AvailableTransactions(initial_transactions))
        }

        // start the wallet scan in a background task
        self.start_wallet_scan_in_task(force_scan)
            .await?
            .await
            .map_err(|error| Error::WalletScanError(error.to_string()))?;

        Produces::ok(())
    }

    pub async fn start_wallet_scan_in_task(&mut self, force_scan: bool) -> ActorResult<()> {
        debug!("start_wallet_scan");

        if let Some(last_scan) = self.last_scan_finished() {
            if elapsed_secs_since(last_scan) < 15 && !force_scan {
                debug!("skipping wallet scan, last scan was less than 15 seconds ago");
                return Produces::ok(());
            }
        }

        // check the node connection, and send frontend the error if it fails
        self.check_node_connection().await?;

        // perform that scanning in a background task
        let addr = self.addr.clone();
        if self
            .wallet
            .metadata
            .internal
            .performed_full_scan_at
            .is_some()
        {
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
                // if less than a minute return the height, without updating
                if elapsed < 60 {
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

        self.wallet
            .switch_mnemonic_to_new_address_type(address_type)?;

        Produces::ok(())
    }

    pub async fn switch_descriptor_to_new_address_type(
        &mut self,
        descriptors: pubport::descriptor::Descriptors,
        address_type: WalletAddressType,
    ) -> ActorResult<()> {
        debug!("actor switch pubkey descriptor wallet");

        self.wallet
            .switch_descriptor_to_new_address_type(descriptors, address_type)?;

        Produces::ok(())
    }

    async fn update_height(&mut self) -> ActorResult<usize> {
        debug!("actor update_height");
        self.check_node_connection().await?;
        let node_client = self.node_client().await?;
        let block_height = node_client
            .get_height()
            .await
            .map_err(|_| Error::GetHeightError)?;

        self.save_last_height_fetched(block_height);
        Produces::ok(block_height)
    }

    pub async fn transaction_details(&mut self, tx_id: TxId) -> ActorResult<TransactionDetails> {
        let tx = self.wallet.bdk.get_tx(tx_id.0).ok_or(Error::TransactionDetailsError(
            "transaction not found".to_string(),
        ))?;

        let labels = self.db.labels.all_labels_for_txn(tx.tx_node.txid)?;
        let details = TransactionDetails::try_new(&self.wallet.bdk, tx, labels.into())
            .map_err(|error| Error::TransactionDetailsError(error.to_string()))?;

        Produces::ok(details)
    }

    // perform full scan in 2 steps:
    // 1. do a full scan of the first 20 addresses, return results
    // 2. do a full scan of the next 150 addresses, return results
    async fn perform_full_scan(&mut self) -> ActorResult<()> {
        self.perform_initial_full_scan().await?;
        Produces::ok(())
    }

    // when a wallet is first opened, we need to scan for its addresses, but we want the
    // initial scan to be fast, so we can have transactions show up in the UI quickly
    // so we do a full scan of only the first 20 addresses, initially
    async fn perform_initial_full_scan(&mut self) -> ActorResult<()> {
        if self.state != ActorState::Initial {
            debug!(
                "already performing scanning or scanned skipping ({:?})",
                self.state
            );

            return Produces::ok(());
        }

        debug!("starting initial full scan");
        self.reconciler
            .send(WalletManagerReconcileMessage::StartedInitialFullScan)
            .unwrap();

        // scan happens in the background, state update afterwards
        self.state = ActorState::PerformingFullScan(FullScanType::Initial);
        self.do_perform_initial_full_scan().await?;

        Produces::ok(())
    }

    // after the initial full scan is complete, do a much for comprehensive scan of the wallet
    // this is slower, but we want to be able to see all transactions in the UI, so scan the next
    // 150 addresses
    async fn perform_expanded_full_scan(&mut self) -> ActorResult<()> {
        if self.state == ActorState::FullScanComplete(FullScanType::Expanded)
            || self.state == ActorState::IncrementalScanComplete
        {
            debug!(
                "already scanned skipping expanded full scan ({:?})",
                self.state
            );
            return Produces::ok(());
        }

        debug!("starting expanded full scan");
        let txns = self.transactions().await?.await?;

        self.reconciler
            .send(WalletManagerReconcileMessage::StartedExpandedFullScan(txns))
            .unwrap();

        // scan happens in the background, state update afterwards
        self.state = ActorState::PerformingFullScan(FullScanType::Expanded);
        send!(self.addr.do_perform_expanded_full_scan());

        Produces::ok(())
    }

    async fn do_perform_initial_full_scan(&mut self) -> ActorResult<()> {
        debug!("do_perform_initial_full_scan");
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
            send!(addr.perform_expanded_full_scan());
        });

        Produces::ok(())
    }

    async fn do_perform_expanded_full_scan(&mut self) -> ActorResult<()> {
        debug!("do_perform_expanded_full_scan");
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
            let scan_result = node_client
                .start_wallet_scan(&graph, full_scan_request, GAP_LIMIT as usize)
                .await;

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
            Database::global()
                .wallets
                .update_internal_metadata(&self.wallet.metadata)?;
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

    /// Mark the wallet as scanned
    /// Notify the frontend that the wallet scan is complete
    /// Ssend the wallet balance and transaction
    async fn notify_scan_complete(&mut self) -> ActorResult<()> {
        use WalletManagerReconcileMessage as Msg;

        // reload the wallet from the file storage
        self.reload_wallet();

        // get and send wallet balance
        let balance = self
            .balance()
            .await?
            .await
            .map_err(|error| Error::WalletBalanceError(error.to_string()))?;

        debug!("sending wallet balance: {balance:?}");
        self.send(Msg::WalletBalanceChanged(balance.into()));

        // get and send transactions
        let transactions: Vec<Transaction> = self
            .transactions()
            .await?
            .await
            .map_err(|error| Error::TransactionsRetrievalError(error.to_string()))?;

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
            .get(
                &self.wallet.id,
                self.wallet.network,
                self.wallet.metadata.wallet_mode,
            )
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
            .get(
                &self.wallet.id,
                self.wallet.network,
                self.wallet.metadata.wallet_mode,
            )
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
            .get(
                &self.wallet.id,
                self.wallet.network,
                self.wallet.metadata.wallet_mode,
            )
            .ok()??;

        let BlockSizeLast {
            block_height,
            last_seen,
        } = &metadata.internal.last_height_fetched?;

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
            .get(
                &self.wallet.id,
                self.wallet.network,
                self.wallet.metadata.wallet_mode,
            )
            .ok()??;

        metadata.internal.last_height_fetched = Some(BlockSizeLast {
            block_height: block_height as u64,
            last_seen: now,
        });

        wallets.update_internal_metadata(&metadata).ok();
        self.wallet.metadata = metadata.clone();

        Some(())
    }

    async fn node_client(&mut self) -> Result<&NodeClient> {
        let node_client = self.node_client.as_ref();
        if node_client.is_none() {
            let node = Database::global().global_config.selected_node();
            let node_client = NodeClient::new(&node).await?;
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
        self.reconciler.send(msg).unwrap();
    }
}

impl Drop for WalletActor {
    fn drop(&mut self) {
        debug!("[DROP] Wallet Actor");
    }
}
