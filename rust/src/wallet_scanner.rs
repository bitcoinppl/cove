use std::time::Instant;

use act_zero::*;
use bdk_chain::bitcoin::Network;
use bdk_wallet::{KeychainKind, Wallet as BdkWallet};
use bip39::Mnemonic;
use crossbeam::channel::Sender;
use pubport::formats::Json;
use strum::IntoEnumIterator as _;
use tracing::{debug, error, info};

/// Default number of addresses to scan
const DEFAULT_SCAN_LIMIT: u32 = 50;

use crate::{
    database::{
        wallet_data::{ScanState, ScanningInfo, WalletDataDb},
        Database,
    },
    keychain::Keychain,
    mnemonic::MnemonicExt,
    node::{
        client::{NodeClient, NodeClientOptions},
        Node,
    },
    task::spawn_actor,
    view_model::wallet::WalletViewModelReconcileMessage,
    wallet::{
        metadata::{DiscoveryState, WalletId, WalletMetadata},
        WalletAddressType, WalletError,
    },
};

#[derive(
    Debug, Default, derive_more::From, derive_more::Into, derive_more::Deref, derive_more::DerefMut,
)]
pub struct Wallets([Option<(WalletAddressType, BdkWallet)>; 3]);

#[derive(
    Debug,
    Clone,
    Default,
    derive_more::From,
    derive_more::Into,
    derive_more::Deref,
    derive_more::DerefMut,
)]
pub struct Workers([Option<WorkerHandle>; 3]);

#[derive(Debug, Clone)]
pub struct WorkerHandle {
    pub id: WalletId,
    pub addr: Addr<WalletScanWorker>,
    pub wallet_type: WalletAddressType,
    pub started_at: Instant,
    pub state: WorkerState,
    pub db: WalletDataDb,
}

#[derive(Debug, Clone, Eq, PartialEq, Copy, Default)]
pub enum WorkerState {
    #[default]
    Created,
    Started,
    FoundAddress,
    NoneFound,
}

#[derive(Debug, Clone, uniffi::Error, thiserror::Error, derive_more::Display)]
pub enum WalletScannerError {
    /// No address types to scan
    NoAddressTypes,

    /// Unable to create wallet
    WalletCreationError(#[from] crate::wallet::WalletError),

    /// No mnemonic available for id {0}
    NoMnemonicAvailable(WalletId),
}

#[derive(Debug, Clone, Eq, PartialEq, uniffi::Enum)]
pub enum ScannerResponse {
    FoundAddresses(Vec<WalletAddressType>),
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
    pub responder: Sender<WalletViewModelReconcileMessage>,
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
        reconciler: Sender<WalletViewModelReconcileMessage>,
    ) -> Result<Self, WalletScannerError> {
        debug!("starting wallet scanner for {}", metadata.id);

        let db = Database::global();
        let network = db.global_config().selected_network().into();

        let id = metadata.id.clone();
        let wallets = match metadata.discovery_state {
            DiscoveryState::StartedJson(json) => Wallets::try_from_json(&json, network)?,
            DiscoveryState::StartedMnemonic => {
                let mnemonic = Keychain::global()
                    .get_wallet_key(&id)
                    .ok()
                    .flatten()
                    .ok_or_else(|| WalletScannerError::NoMnemonicAvailable(id.clone()))?;

                Wallets::try_from_mnemonic(&mnemonic, network)?
            }
            DiscoveryState::Single
            | DiscoveryState::NoneFound
            | DiscoveryState::ChoseAdressType
            | DiscoveryState::FoundAddresses(_) => return Err(WalletScannerError::NoAddressTypes),
        };

        if wallets.iter().all(Option::is_none) {
            return Err(WalletScannerError::NoAddressTypes);
        }

        let node = db.global_config().selected_node();
        let options = NodeClientOptions {
            batch_size: 1,
            stop_gap: 50,
        };

        let client_builder = NodeClientBuilder { node, options };
        Ok(Self::new(metadata.id, client_builder, wallets, reconciler))
    }

    pub fn new(
        id: WalletId,
        node_client_builder: NodeClientBuilder,
        wallets: Wallets,
        reconciler: Sender<WalletViewModelReconcileMessage>,
    ) -> Self {
        let mut started_workers = 0;
        let mut workers = Workers::default();

        // create workers
        for (wallet_type, wallet) in wallets.0.into_iter().flatten() {
            let worker =
                WalletScanWorker::new(id.clone(), wallet_type, wallet, node_client_builder.clone());

            let addr = spawn_actor(worker);
            workers[wallet_type.index()].replace(WorkerHandle {
                id: id.clone(),
                addr,
                wallet_type,
                started_at: Instant::now(),
                state: WorkerState::Created,
                db: WalletDataDb::new(id.clone()),
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

        self.workers[wallet_type.index()]
            .as_mut()
            .expect("worker started")
            .state = WorkerState::FoundAddress;

        let any_still_running = self.workers.iter().any(|worker| {
            worker
                .as_ref()
                .map_or(false, |worker| worker.state == WorkerState::Started)
        });

        // all workers are done, send the response
        if !any_still_running {
            let found_addresses = self
                .workers
                .iter()
                .filter_map(|worker| {
                    worker
                        .as_ref()
                        .filter(|worker| worker.state == WorkerState::FoundAddress)
                        .map(|worker| worker.wallet_type)
                })
                .collect::<Vec<WalletAddressType>>();

            self.responder
                .send(ScannerResponse::FoundAddresses(found_addresses.clone()).into())?;

            // update wallet metadata
            self.set_metadata(DiscoveryState::FoundAddresses(found_addresses))?;

            return Produces::ok(());
        }

        Produces::ok(())
    }

    pub async fn mark_limit_reached(&mut self, wallet_type: WalletAddressType) -> ActorResult<()> {
        info!("marked worker {wallet_type:?} limit reached");

        self.workers[wallet_type.index()]
            .as_mut()
            .expect("worker started")
            .state = WorkerState::NoneFound;

        let any_still_running = self.workers.iter().any(|worker| {
            worker
                .as_ref()
                .map_or(false, |worker| worker.state == WorkerState::Started)
        });

        // all workers are done, send the response
        if !any_still_running {
            let found_addresses = self
                .workers
                .iter()
                .filter_map(|worker| {
                    worker
                        .as_ref()
                        .filter(|worker| worker.state == WorkerState::FoundAddress)
                        .map(|worker| worker.wallet_type)
                })
                .collect::<Vec<WalletAddressType>>();

            if found_addresses.is_empty() {
                self.responder.send(ScannerResponse::NoneFound.into())?;

                self.set_metadata(DiscoveryState::NoneFound)?;
            } else {
                self.responder
                    .send(ScannerResponse::FoundAddresses(found_addresses.clone()).into())?;

                self.set_metadata(DiscoveryState::FoundAddresses(found_addresses))?;
            }

            return Produces::ok(());
        }

        Produces::ok(())
    }

    fn set_metadata(&mut self, discovery_state: DiscoveryState) -> ActorResult<()> {
        debug!("setting wallet metadata: {discovery_state:?}");
        let network = Database::global().global_config.selected_network();
        let db = Database::global().wallets();

        let Ok(Some(mut metadata)) = db.get(&self.id, network) else {
            error!("wallet metadata not found");
            return Produces::ok(());
        };

        metadata.discovery_state = discovery_state.into();
        db.save_wallet(metadata)?;

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
        let db = WalletDataDb::new(id.clone());

        let scan_info = db
            .get_scan_state(wallet_type)
            .ok()
            .flatten()
            .map(|scan_state| match scan_state {
                ScanState::Scanning(info) => info,
                _ => ScanningInfo::new(wallet_type),
            })
            .unwrap_or_else(|| ScanningInfo::new(wallet_type));

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
        let mut current_address = self.scan_info.count;
        let client = self.client_builder.build().await?;

        loop {
            let wallet_type = self.wallet_type;

            let address = self
                .wallet
                .peek_address(KeychainKind::External, current_address);

            // found address
            if client.check_address_for_txn(address.address).await? {
                call!(self.parent.mark_found_txn(wallet_type));

                // save the scan state
                self.db.set_scan_state(wallet_type, ScanState::Completed)?;

                return Produces::ok(());
            }

            current_address += 1;
            debug!("checked {current_address} addresses for {wallet_type}");

            // every 5 addresses, save the scan state
            if current_address % 5 == 0 {
                let scan_state = ScanningInfo {
                    address_type: wallet_type,
                    count: current_address,
                };

                self.db.set_scan_state(wallet_type, scan_state)?;
            }

            // scanning is done, no adddress found
            if current_address >= self.scan_limit {
                self.db.set_scan_state(wallet_type, ScanState::Completed)?;
                call!(self.parent.mark_limit_reached(wallet_type));
            }
        }
    }
}

impl WalletAddressType {
    pub fn index(&self) -> usize {
        match self {
            WalletAddressType::NativeSegwit => 0_usize,
            WalletAddressType::WrappedSegwit => 1,
            WalletAddressType::Legacy => 2,
        }
    }
}

impl Wallets {
    pub fn try_from_json(json: &Json, network: Network) -> Result<Self, WalletError> {
        let mut wallets = Self::default();

        for (json, type_) in [
            (&json.bip84, WalletAddressType::NativeSegwit),
            (&json.bip49, WalletAddressType::WrappedSegwit),
            (&json.bip44, WalletAddressType::Legacy),
        ] {
            if let Some(json) = json {
                let params = BdkWallet::create(json.external.clone(), json.internal.clone())
                    .network(network);

                let wallet = BdkWallet::create_with_params(params)
                    .map_err(|error| WalletError::BdkError(error.to_string()))?;

                wallets[type_.index()] = Some((type_, wallet));
            }
        }

        Ok(wallets)
    }

    pub fn try_from_mnemonic(mnemonic: &Mnemonic, network: Network) -> Result<Self, WalletError> {
        let mut wallets = Wallets::default();

        for type_ in WalletAddressType::iter() {
            let descriptor = mnemonic.clone().into_descriptors(None, network, type_);
            let wallet = BdkWallet::create(
                descriptor.external.to_tuple(),
                descriptor.internal.to_tuple(),
            )
            .network(network)
            .create_wallet_no_persist()
            .map_err(|error| WalletError::BdkError(error.to_string()))?;

            wallets[type_.index()] = Some((type_, wallet));
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

impl From<ScannerResponse> for WalletViewModelReconcileMessage {
    fn from(response: ScannerResponse) -> Self {
        WalletViewModelReconcileMessage::WalletScannerResponse(response)
    }
}