use std::{sync::Arc, time::Instant};

use act_zero::*;
use bdk_chain::bitcoin::{Address, Network};
use bdk_wallet::{KeychainKind, Wallet as BdkWallet};
use bip39::Mnemonic;
use crossbeam::channel::Sender;
use eyre::Context;
use pubport::formats::Json;
use tracing::{debug, error, info, warn};

/// Default number of addresses to scan
const DEFAULT_SCAN_LIMIT: u32 = 150;

use crate::{
    database::{
        wallet_data::{ScanState, ScanningInfo, WalletDataDb},
        Database,
    },
    keychain::Keychain,
    manager::wallet::WalletManagerReconcileMessage,
    mnemonic::MnemonicExt,
    node::{
        client::{NodeClient, NodeClientOptions},
        Node,
    },
    task::spawn_actor,
    wallet::{
        metadata::{DiscoveryState, FoundAddress, FoundJson, WalletId, WalletMetadata},
        WalletAddressType, WalletError,
    },
};

#[derive(
    Debug, Default, derive_more::From, derive_more::Into, derive_more::Deref, derive_more::DerefMut,
)]
pub struct Wallets([Option<(WalletAddressType, BdkWallet)>; 2]);

#[derive(
    Debug,
    Clone,
    Default,
    derive_more::From,
    derive_more::Into,
    derive_more::Deref,
    derive_more::DerefMut,
)]
pub struct Workers([Option<WorkerHandle>; 2]);

#[derive(Debug, Clone)]
pub struct WorkerHandle {
    pub id: WalletId,
    pub addr: Addr<WalletScanWorker>,
    pub wallet_type: WalletAddressType,
    pub started_at: Instant,
    pub state: WorkerState,
    pub db: WalletDataDb,
}

#[derive(Debug, Clone, Eq, PartialEq, Default)]
pub enum WorkerState {
    #[default]
    Created,
    Started,
    FoundAddress(String),
    NoneFound,
}

#[derive(Debug, Clone, uniffi::Error, thiserror::Error)]
pub enum WalletScannerError {
    #[error("No address types to scan")]
    NoAddressTypes,

    #[error("Unable to create wallet")]
    WalletCreationError(#[from] crate::wallet::WalletError),

    #[error("No mnemonic available for id {0}")]
    NoMnemonicAvailable(WalletId),
}

#[derive(Debug, Clone, Eq, PartialEq, uniffi::Enum)]
pub enum ScannerResponse {
    FoundAddresses(Vec<FoundAddress>),
    NoneFound,
}

#[derive(Debug, Clone)]
pub struct NodeClientBuilder {
    pub node: Node,
    pub options: NodeClientOptions,
}

#[derive(Debug, Clone)]
pub struct WalletScanner {
    pub id: WalletId,
    pub addr: WeakAddr<Self>,
    pub workers: Workers,
    pub started_at: Instant,
    pub node_client_builder: NodeClientBuilder,
    pub scan_source: ScanSource,
    pub responder: Sender<WalletManagerReconcileMessage>,
}

#[derive(Debug, Clone)]
pub enum ScanSource {
    Json(Arc<FoundJson>),
    Mnemonic,
}

#[async_trait::async_trait]
impl Actor for WalletScanner {
    async fn started(&mut self, addr: Addr<Self>) -> ActorResult<()> {
        self.addr = addr.downgrade();
        self.started_at = Instant::now();

        // start workers
        self.start_workers().await?;

        Produces::ok(())
    }

    async fn error(&mut self, error: ActorError) -> bool {
        error!("WalletScanner Error: {error:?}");
        false
    }
}

impl WalletScanner {
    /// Create a new scanner based on the DiscoveryState of the wallet metadata
    /// Only create a scanner if the discovery state is not already completed, and if
    /// have the required information to start a scan.
    pub fn try_new(
        metadata: WalletMetadata,
        reconciler: Sender<WalletManagerReconcileMessage>,
    ) -> Result<Self, WalletScannerError> {
        debug!(
            "starting wallet scanner for {}, metadata: {metadata:?}",
            metadata.id
        );

        let db = Database::global();
        let network = db.global_config().selected_network().into();

        let id = metadata.id.clone();
        let (wallets, scan_source) = match metadata.discovery_state {
            DiscoveryState::StartedJson(json) => (
                Wallets::try_from_json(&json, network)?,
                ScanSource::Json(json),
            ),
            DiscoveryState::StartedMnemonic => {
                let mnemonic = Keychain::global()
                    .get_wallet_key(&id)
                    .ok()
                    .flatten()
                    .ok_or_else(|| WalletScannerError::NoMnemonicAvailable(id.clone()))?;

                (
                    Wallets::try_from_mnemonic(&mnemonic, network)?,
                    ScanSource::Mnemonic,
                )
            }
            DiscoveryState::Single
            | DiscoveryState::NoneFound
            | DiscoveryState::ChoseAdressType
            | DiscoveryState::FoundAddressesFromJson(_, _)
            | DiscoveryState::FoundAddressesFromMnemonic(_) => {
                return Err(WalletScannerError::NoAddressTypes)
            }
        };

        if wallets.iter().all(Option::is_none) {
            return Err(WalletScannerError::NoAddressTypes);
        }

        let node = db.global_config().selected_node();
        let options = NodeClientOptions { batch_size: 1 };

        let client_builder = NodeClientBuilder { node, options };
        Ok(Self::new(
            metadata.id,
            client_builder,
            wallets,
            scan_source,
            reconciler,
        ))
    }

    pub fn new(
        id: WalletId,
        node_client_builder: NodeClientBuilder,
        wallets: Wallets,
        scan_source: ScanSource,
        reconciler: Sender<WalletManagerReconcileMessage>,
    ) -> Self {
        let mut started_workers = 0;
        let mut workers = Workers::default();

        // create workers
        for (wallet_type, wallet) in wallets.0.into_iter().flatten() {
            let worker =
                WalletScanWorker::new(id.clone(), wallet_type, wallet, node_client_builder.clone());

            let addr = spawn_actor(worker);
            workers[index(wallet_type)].replace(WorkerHandle {
                id: id.clone(),
                addr,
                wallet_type,
                started_at: Instant::now(),
                state: WorkerState::Created,
                db: WalletDataDb::new_or_existing(id.clone()),
            });

            started_workers += 1;
        }

        info!("started {started_workers} workers");

        Self {
            id,
            addr: Default::default(),
            workers,
            started_at: Instant::now(),
            node_client_builder,
            scan_source,
            responder: reconciler,
        }
    }

    async fn start_workers(&mut self) -> ActorResult<()> {
        let parent = self.addr.clone();

        for worker in self.workers.iter_mut().flatten() {
            call!(worker.addr.start(parent.clone())).await?;
            worker.state = WorkerState::Started;
        }

        Produces::ok(())
    }

    pub async fn mark_found_txn(&mut self, wallet_type: WalletAddressType) -> ActorResult<()> {
        info!("marked worker {wallet_type:?} as found");

        //  mark as found and stop the worker
        let worker = self.workers[index(wallet_type)]
            .as_mut()
            .expect("worker started");

        let address = call!(worker.addr.first_address()).await?;
        worker.state = WorkerState::FoundAddress(address);
        worker.addr = Default::default();

        let any_still_running = self.workers.iter().any(|worker| {
            worker
                .as_ref()
                .is_some_and(|worker| worker.state == WorkerState::Started)
        });

        // all workers are done, send the response
        if !any_still_running {
            let found_addresses = self.found_addresses();

            self.responder
                .send(ScannerResponse::FoundAddresses(found_addresses.clone()).into())?;

            // update wallet metadata
            self.set_metadata_address_found()?;

            return Produces::ok(());
        }

        Produces::ok(())
    }

    pub async fn mark_limit_reached(&mut self, wallet_type: WalletAddressType) -> ActorResult<()> {
        info!("marked worker {wallet_type:?} limit reached");

        let worker = self.workers[index(wallet_type)]
            .as_mut()
            .expect("worker started");

        worker.state = WorkerState::NoneFound;
        worker.addr = Default::default();

        let any_still_running = self.workers.iter().any(|worker| {
            worker
                .as_ref()
                .is_some_and(|worker| worker.state == WorkerState::Started)
        });

        // all workers are done, send the response
        if !any_still_running {
            let found_addresses = self.found_addresses();

            if found_addresses.is_empty() {
                self.responder.send(ScannerResponse::NoneFound.into())?;
                self.set_metadata(DiscoveryState::NoneFound)?;
            } else {
                self.responder
                    .send(ScannerResponse::FoundAddresses(found_addresses.clone()).into())?;

                self.set_metadata_address_found()?;
            }

            return Produces::ok(());
        }

        Produces::ok(())
    }

    fn found_addresses(&self) -> Vec<FoundAddress> {
        self.workers
            .iter()
            .filter_map(|worker| {
                worker
                    .as_ref()
                    .filter(|worker| matches!(worker.state, WorkerState::FoundAddress(_)))
                    .map(|worker| {
                        let WorkerState::FoundAddress(first_address) = worker.state.clone() else {
                            panic!("impossible")
                        };

                        FoundAddress {
                            type_: worker.wallet_type,
                            first_address,
                        }
                    })
            })
            .collect::<Vec<FoundAddress>>()
    }

    fn set_metadata_address_found(&mut self) -> ActorResult<()> {
        match &self.scan_source {
            ScanSource::Json(json) => {
                self.set_metadata(DiscoveryState::FoundAddressesFromJson(
                    self.found_addresses(),
                    json.clone(),
                ))?;
            }

            ScanSource::Mnemonic => {
                self.set_metadata(DiscoveryState::FoundAddressesFromMnemonic(
                    self.found_addresses(),
                ))?;
            }
        }

        Produces::ok(())
    }

    fn set_metadata(&mut self, discovery_state: DiscoveryState) -> ActorResult<()> {
        debug!("setting wallet metadata: {discovery_state:?}");
        let network = Database::global().global_config.selected_network();
        let mode = Database::global().global_config.wallet_mode();
        let db = Database::global().wallets();

        let Ok(Some(mut metadata)) = db.get(&self.id, network, mode) else {
            error!("wallet metadata not found");
            return Produces::ok(());
        };

        metadata.discovery_state = discovery_state;
        db.update_wallet_metadata(metadata.clone())?;

        self.responder
            .send(WalletManagerReconcileMessage::WalletMetadataChanged(
                metadata,
            ))?;

        Produces::ok(())
    }
}

// WORKER

#[derive(Debug)]
pub struct WalletScanWorker {
    parent: WeakAddr<WalletScanner>,
    addr: WeakAddr<Self>,
    client_builder: NodeClientBuilder,
    wallet_type: WalletAddressType,
    wallet: BdkWallet,
    started_at: Instant,
    scan_info: ScanningInfo,
    db: WalletDataDb,
    scan_limit: u32,
}

#[async_trait::async_trait]
impl Actor for WalletScanWorker {
    async fn started(&mut self, addr: Addr<Self>) -> ActorResult<()> {
        self.addr = addr.downgrade();
        self.started_at = Instant::now();

        Produces::ok(())
    }

    async fn error(&mut self, error: ActorError) -> bool {
        error!("WalletScanWorker Error: {error:?}");
        false
    }
}

impl WalletScanWorker {
    pub fn new(
        id: WalletId,
        wallet_type: WalletAddressType,
        wallet: BdkWallet,
        client_builder: NodeClientBuilder,
    ) -> Self {
        debug!("creating wallet scanner for {id}, type: {wallet_type}");
        let db = WalletDataDb::new_or_existing(id.clone());

        let scan_info = db
            .get_scan_state(wallet_type)
            .ok()
            .flatten()
            .map(|scan_state| match scan_state {
                ScanState::Scanning(info) => info,
                ScanState::Completed => {
                    warn!("trying to scan completed wallet");
                    ScanningInfo::new(wallet_type)
                }
                _ => ScanningInfo::new(wallet_type),
            })
            .unwrap_or_else(|| ScanningInfo::new(wallet_type));

        debug!("wallet scan info: {scan_info:?}");
        Self {
            parent: Default::default(),
            addr: Default::default(),
            wallet,
            client_builder,
            wallet_type,
            started_at: Instant::now(),
            scan_info,
            db,
            scan_limit: DEFAULT_SCAN_LIMIT,
        }
    }

    pub async fn start(&mut self, parent: WeakAddr<WalletScanner>) {
        self.parent = parent;

        let addr = self.addr.clone();

        // start the scan and return immediately
        send!(addr.start_scan());
    }

    async fn start_scan(&mut self) -> ActorResult<()> {
        let client = self.client_builder.build().await?;
        let wallet_type = self.wallet_type;

        let addr = self.addr.clone();
        let scan_limit = self.scan_limit;
        let current_address = self.scan_info.count;
        let parent = self.parent.clone();
        let db = self.db.clone();

        self.addr.send_fut(async move {
            let run_with_error = || async move {
                let mut current_address = current_address;

                loop {
                    let address = call!(addr.address_at(current_address)).await?;

                    // found address
                    if client
                        .check_address_for_txn(address)
                        .await
                        .context("could not check address")?
                    {
                        call!(parent.mark_found_txn(wallet_type)).await?;

                        // save the scan state
                        db.set_scan_state(wallet_type, ScanState::Completed)
                            .context("save scan state")?;

                        break;
                    }

                    current_address += 1;
                    debug!("checked {current_address} addresses for {wallet_type}");

                    // every 5 addresses, save the scan state
                    if current_address % 5 == 0 {
                        let scan_state = ScanningInfo {
                            address_type: wallet_type,
                            count: current_address,
                        };

                        if let Err(error) = db.set_scan_state(wallet_type, scan_state) {
                            error!("unable to update scan state: {error}");
                        };
                    };

                    if current_address >= scan_limit {
                        db.set_scan_state(wallet_type, ScanState::Completed)
                            .expect("save scan state");

                        call!(parent.mark_limit_reached(wallet_type));
                        break;
                    }
                }

                Ok::<(), eyre::Error>(())
            };

            if let Err(error) = run_with_error().await {
                error!("wallet scan failed: {error}");
                // todo: maybe send the error back to the parent? the scanner or the view model?
            }
        });

        Produces::ok(())
    }

    pub async fn first_address(&self) -> ActorResult<String> {
        let Produces::Value(address) = self.address_at(0).await? else {
            panic!("impossible");
        };

        Produces::ok(address.to_string())
    }

    async fn address_at(&self, index: u32) -> ActorResult<Address> {
        let address = self
            .wallet
            .peek_address(KeychainKind::External, index)
            .address;

        Produces::ok(address)
    }
}

fn index(type_: WalletAddressType) -> usize {
    match type_ {
        WalletAddressType::WrappedSegwit => 0,
        WalletAddressType::Legacy => 1,
        WalletAddressType::NativeSegwit => panic!("Not scanning the default one NativeSegwit"),
    }
}

impl Wallets {
    pub fn try_from_json(json: &Json, network: Network) -> Result<Self, WalletError> {
        let mut wallets = Self::default();

        for (json, type_) in [
            (&json.bip49, WalletAddressType::WrappedSegwit),
            (&json.bip44, WalletAddressType::Legacy),
        ] {
            if let Some(json) = json {
                let params = BdkWallet::create(json.external.clone(), json.internal.clone())
                    .network(network);

                let wallet = BdkWallet::create_with_params(params)
                    .map_err(|error| WalletError::BdkError(error.to_string()))?;

                wallets[index(type_)] = Some((type_, wallet));
            }
        }

        Ok(wallets)
    }

    pub fn try_from_mnemonic(mnemonic: &Mnemonic, network: Network) -> Result<Self, WalletError> {
        let mut wallets = Wallets::default();

        for type_ in [WalletAddressType::WrappedSegwit, WalletAddressType::Legacy] {
            let descriptor = mnemonic.clone().into_descriptors(None, network, type_);
            let wallet = BdkWallet::create(
                descriptor.external.into_tuple(),
                descriptor.internal.into_tuple(),
            )
            .network(network)
            .create_wallet_no_persist()
            .map_err(|error| WalletError::BdkError(error.to_string()))?;

            wallets[index(type_)] = Some((type_, wallet));
        }

        Ok(wallets)
    }
}

impl NodeClientBuilder {
    pub async fn build(&self) -> Result<NodeClient, crate::node::client::Error> {
        let node_client = NodeClient::new_with_options(&self.node, self.options).await?;
        Ok(node_client)
    }
}

impl From<ScannerResponse> for WalletManagerReconcileMessage {
    fn from(response: ScannerResponse) -> Self {
        WalletManagerReconcileMessage::WalletScannerResponse(response)
    }
}
