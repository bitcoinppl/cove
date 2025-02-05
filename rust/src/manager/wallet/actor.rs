use crate::{
    database::Database,
    manager::wallet::{Error, SendFlowErrorAlert, WalletManagerError},
    mnemonic,
    node::client::NodeClient,
    transaction::{fees::BdkFeeRate, FeeRate, Transaction, TransactionDetails, TxId},
    wallet::{
        balance::Balance,
        confirm::{AddressAndAmount, ConfirmDetails, InputOutputDetails, SplitOutput},
        metadata::BlockSizeLast,
        Address, AddressInfo, Wallet, WalletAddressType,
    },
};
use act_zero::*;
use bdk_chain::{
    bitcoin::Psbt,
    spk_client::{FullScanResponse, SyncResponse},
};
use bdk_wallet::{KeychainKind, TxOrdering};
use bitcoin::Amount;
use bitcoin::{params::Params, Transaction as BdkTransaction};
use crossbeam::channel::Sender;
use eyre::Context as _;
use std::time::{Duration, UNIX_EPOCH};
use tap::TapFallible as _;
use tracing::{debug, error, info};

use self::mnemonic::{Mnemonic, MnemonicExt as _};

use super::WalletManagerReconcileMessage;

#[derive(Debug)]
pub struct WalletActor {
    pub addr: WeakAddr<Self>,
    pub reconciler: Sender<WalletManagerReconcileMessage>,
    pub wallet: Wallet,
    pub node_client: Option<NodeClient>,

    last_scan_finished_: Option<Duration>,
    last_height_fetched_: Option<(Duration, usize)>,

    pub state: ActorState,
}

#[derive(Debug)]
pub enum ActorState {
    Initial,
    PerformingFullScan,
    PerformingIncrementalScan,
    ScanComplete,
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
        Self {
            addr: Default::default(),
            reconciler,
            wallet,
            node_client: None,
            last_scan_finished_: None,
            last_height_fetched_: None,
            state: ActorState::Initial,
        }
    }

    pub async fn balance(&mut self) -> ActorResult<Balance> {
        let balance = self.wallet.balance();
        Produces::ok(balance)
    }

    pub async fn build_drain_tx(&mut self, address: Address, fee: FeeRate) -> ActorResult<Psbt> {
        let script_pubkey = address.script_pubkey();
        let mut tx_builder = self.wallet.build_tx();

        tx_builder
            .drain_wallet()
            .drain_to(script_pubkey)
            .fee_rate(fee.into());

        let psbt = tx_builder.finish()?;
        Produces::ok(psbt)
    }

    pub async fn build_tx(
        &mut self,
        amount: Amount,
        address: Address,
        fee_rate: BdkFeeRate,
    ) -> ActorResult<Psbt> {
        let script_pubkey = address.script_pubkey();

        let mut tx_builder = self.wallet.build_tx();
        tx_builder.ordering(TxOrdering::Untouched);
        tx_builder.add_recipient(script_pubkey, amount);
        tx_builder.fee_rate(fee_rate);

        let psbt = tx_builder.finish()?;

        Produces::ok(psbt)
    }

    pub async fn transactions(&mut self) -> ActorResult<Vec<Transaction>> {
        let zero = Amount::ZERO.into();
        let mut transactions = self
            .wallet
            .transactions()
            .map(|tx| Transaction::new(&self.wallet, tx))
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
            .filter(|output| !self.wallet.is_mine(output.address.script_pubkey()))
            .cloned()
            .collect();

        let internal = outputs
            .iter()
            .filter(|output| self.wallet.is_mine(output.address.script_pubkey()))
            .cloned()
            .collect();

        Produces::ok(SplitOutput { external, internal })
    }

    pub async fn get_confirm_details(
        &mut self,
        psbt: Psbt,
        fee_rate: BdkFeeRate,
    ) -> ActorResult<ConfirmDetails> {
        #[inline(always)]
        fn error(s: &str) -> Box<dyn std::error::Error + Send + Sync + 'static> {
            WalletManagerError::GetConfirmDetailsError(s.to_string()).into()
        }

        let external_outputs = psbt
            .unsigned_tx
            .output
            .iter()
            .filter(|output| !self.wallet.is_mine(output.script_pubkey.clone()))
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

        let params = Params::from(self.wallet.network());
        let sending_to = bitcoin::Address::from_script(&output.script_pubkey, params)
            .context("unable to get address from script")?;

        let sending_amount = output.value;
        let fee = psbt.fee()?;
        let spending_amount = sending_amount
            .checked_add(fee)
            .ok_or_else(|| error("fee overflow, cannot calculate spending amount"))?;

        let network = self.wallet.network();
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

        Produces::ok(details)
    }

    pub async fn sign_and_broadcast_transaction(&mut self, mut psbt: Psbt) -> ActorResult<()> {
        fn err(s: &str) -> Box<dyn std::error::Error + Send + Sync + 'static> {
            Error::SignAndBroadcastError(s.to_string()).into()
        }

        // TODO: temporary, remove to allow sending on mainnet
        if self.wallet.network == crate::network::Network::Bitcoin {
            return Err(err("sending on mainnet not supported yet"));
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

        self.broadcast_transaction(transaction).await?;

        Produces::ok(())
    }

    pub async fn broadcast_transaction(&mut self, transaction: BdkTransaction) -> ActorResult<()> {
        fn err(s: &str) -> Box<dyn std::error::Error + Send + Sync + 'static> {
            Error::SignAndBroadcastError(s.to_string()).into()
        }

        self.node_client
            .as_ref()
            .ok_or_else(|| err("node client not set"))?
            .broadcast_transaction(transaction)
            .await
            .map_err(|_error| err("failed to broadcast transaction, try again"))?;

        Produces::ok(())
    }

    pub async fn address_at(&mut self, index: u32) -> ActorResult<AddressInfo> {
        let address = self.wallet.peek_address(KeychainKind::External, index);
        Produces::ok(address.into())
    }

    pub async fn next_address(&mut self) -> ActorResult<AddressInfo> {
        let address = self.wallet.get_next_address()?;
        Produces::ok(address)
    }

    pub async fn check_node_connection(&mut self) -> ActorResult<()> {
        let node_client = match &self.node_client {
            Some(node_client) => node_client,
            None => {
                let node = Database::global().global_config.selected_node();
                let node_client = NodeClient::new(&node).await?;
                self.node_client = Some(node_client);

                self.node_client.as_ref().expect("just checked")
            }
        };

        node_client
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
        use WalletManagerReconcileMessage as Msg;
        debug!("start_wallet_scan");

        if !force_scan {
            if let Some(last_scan) = self.last_scan_finished() {
                if elapsed_secs_since(last_scan) < 60 {
                    info!("skipping wallet scan, last scan was less than 60 seconds ago");
                    return Produces::ok(());
                }
            }
        }

        self.reconciler.send(Msg::StartedWalletScan).unwrap();

        let node = Database::global().global_config.selected_node();
        let reconciler = self.reconciler.clone();

        // save the node client
        match NodeClient::new(&node).await {
            Ok(client) => {
                self.node_client = Some(client);
            }
            Err(error) => {
                reconciler
                    .send(Msg::NodeConnectionFailed(error.to_string()))
                    .unwrap();

                return Err(error.into());
            }
        }

        assert!(self.node_client.is_some());

        // check the node connection, and send frontend the error if it fails
        send!(self.addr.check_node_connection());

        // perform that scanning in a background task
        let addr = self.addr.clone();
        if self.wallet.metadata.performed_full_scan {
            send!(addr.perform_incremental_scan());
        } else {
            send!(addr.perform_full_scan());
        }

        Produces::ok(())
    }

    pub async fn get_height(&mut self, force: bool) -> ActorResult<usize> {
        if !force {
            if let Some((last_height_fetched, block_height)) = self.last_height_fetched() {
                let elapsed = elapsed_secs_since(last_height_fetched);
                if elapsed < 60 * 5 {
                    if elapsed < 60 {
                        return Produces::ok(block_height);
                    }

                    send!(self.addr.update_height());
                    return Produces::ok(block_height);
                }
            }
        }

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
        let node_client = self
            .node_client
            .as_ref()
            .ok_or(eyre::eyre!("node client not set"))?;

        let block_height = node_client
            .get_height()
            .await
            .map_err(|_| Error::GetHeightError)?;

        self.set_last_height_fetched(block_height);
        Produces::ok(block_height)
    }

    pub async fn transaction_details(&mut self, tx_id: TxId) -> ActorResult<TransactionDetails> {
        let tx = self
            .wallet
            .get_tx(tx_id.0)
            .ok_or(Error::TransactionDetailsError(
                "transaction not found".to_string(),
            ))?;

        let details = TransactionDetails::try_new(&self.wallet, tx)
            .map_err(|error| Error::TransactionDetailsError(error.to_string()))?;

        Produces::ok(details)
    }

    async fn perform_full_scan(&mut self) -> ActorResult<()> {
        debug!("starting full scan");

        self.state = ActorState::PerformingFullScan;
        let start = UNIX_EPOCH.elapsed().unwrap().as_secs();

        let full_scan_request = self.wallet.start_full_scan().build();

        let graph = self.wallet.tx_graph().clone();
        let node_client = self
            .node_client
            .clone()
            .as_ref()
            .ok_or(eyre::eyre!("node client not set"))?
            .clone();

        let addr = self.addr.clone();
        self.addr.send_fut(async move {
            let full_scan_result = node_client
                .start_wallet_scan(&graph, full_scan_request)
                .await;

            let now = UNIX_EPOCH.elapsed().unwrap().as_secs();
            debug!("done full scan in {}s", now - start);

            // update wallet state
            send!(addr.handle_full_scan_complete(full_scan_result));
        });

        Produces::ok(())
    }

    async fn perform_incremental_scan(&mut self) -> ActorResult<()> {
        debug!("starting incremental scan");
        self.state = ActorState::PerformingIncrementalScan;

        let start = UNIX_EPOCH.elapsed().unwrap().as_secs();

        let scan_request = self.wallet.start_sync_with_revealed_spks().build();
        let graph = self.wallet.tx_graph().clone();
        let node_client = self
            .node_client
            .as_ref()
            .ok_or(eyre::eyre!("node client not set"))?
            .clone();

        let addr = self.addr.clone();
        self.addr.send_fut(async move {
            let sync_result = node_client.sync(&graph, scan_request).await;
            let now = UNIX_EPOCH.elapsed().unwrap().as_secs();
            debug!("done incremental scan in {}s", now - start);

            // update wallet state
            send!(addr.handle_incremental_scan_complete(sync_result));
        });

        Produces::ok(())
    }

    async fn handle_full_scan_complete(
        &mut self,
        full_scan_result: Result<FullScanResponse<KeychainKind>, crate::node::client::Error>,
    ) -> ActorResult<()> {
        debug!("applying full scan result");

        let full_scan_result = full_scan_result?;

        self.wallet.apply_update(full_scan_result)?;
        self.wallet.persist()?;
        self.set_last_scan_finished();

        self.wallet.metadata.performed_full_scan = true;
        Database::global()
            .wallets
            .update_wallet_metadata(self.wallet.metadata.clone())?;

        self.mark_and_notify_scan_complete().await?;

        Produces::ok(())
    }

    async fn handle_incremental_scan_complete(
        &mut self,
        sync_result: Result<SyncResponse, crate::node::client::Error>,
    ) -> ActorResult<()> {
        let sync_result = sync_result?;
        self.wallet.apply_update(sync_result)?;
        self.wallet.persist()?;
        self.set_last_scan_finished();

        self.mark_and_notify_scan_complete().await?;

        Produces::ok(())
    }

    /// Mark the wallet as scanned
    /// Notify the frontend that the wallet scan is complete
    /// Ssend the wallet balance and transactions
    async fn mark_and_notify_scan_complete(&mut self) -> ActorResult<()> {
        use WalletManagerReconcileMessage as Msg;

        // set the scan state to complete
        self.state = ActorState::ScanComplete;

        // get and send wallet balance
        let balance = self
            .balance()
            .await?
            .await
            .map_err(|error| Error::WalletBalanceError(error.to_string()))?;

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

    fn last_scan_finished(&mut self) -> Option<Duration> {
        if let Some(last_scan_finished) = self.last_scan_finished_ {
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

        let last_scan_finished = metadata.internal().last_scan_finished;
        self.last_scan_finished_ = last_scan_finished;

        last_scan_finished
    }

    fn set_last_scan_finished(&mut self) -> Option<()> {
        let now = UNIX_EPOCH.elapsed().unwrap();
        self.last_scan_finished_ = Some(now);

        let wallets = Database::global().wallets();

        let mut metadata = wallets
            .get(
                &self.wallet.id,
                self.wallet.network,
                self.wallet.metadata.wallet_mode,
            )
            .ok()??;
        metadata.internal_mut().last_scan_finished = Some(now);

        wallets.create_wallet(metadata).ok()
    }

    fn last_height_fetched(&mut self) -> Option<(Duration, usize)> {
        if let Some(last_height_fetched) = self.last_height_fetched_ {
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
        } = &metadata.internal().last_height_fetched?;

        let last_height_fetched = Some((*last_seen, *(block_height) as usize));
        self.last_height_fetched_ = last_height_fetched;

        last_height_fetched
    }

    fn set_last_height_fetched(&mut self, block_height: usize) -> Option<()> {
        let now = UNIX_EPOCH.elapsed().unwrap();
        self.last_height_fetched_ = Some((now, block_height));

        let wallets = Database::global().wallets();
        let mut metadata = wallets
            .get(
                &self.wallet.id,
                self.wallet.network,
                self.wallet.metadata.wallet_mode,
            )
            .ok()??;

        metadata.internal_mut().last_height_fetched = Some(BlockSizeLast {
            block_height: block_height as u64,
            last_seen: now,
        });

        wallets.create_wallet(metadata).ok()
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
