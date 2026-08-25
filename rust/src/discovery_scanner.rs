//! Discovers which imported wallet address types have transaction history
//!
//! This scanner runs during import flows when metadata is in a discovery state such as
//! `StartedJson`, `StartedMnemonic`, or `StartedXprv`. It checks legacy and wrapped SegWit candidate wallets
//! until it finds history or reaches the discovery scan limit, then records the discovered
//! address types in wallet metadata. It is separate from the selected-wallet progressive
//! transaction scanner, which syncs transactions for an already selected wallet

use std::{sync::Arc, time::Instant};

use act_zero::*;
use bdk_wallet::{
    KeychainKind, Wallet as BdkWallet,
    bitcoin::{Address, Network},
    descriptor::ExtendedDescriptor,
};
use cove_device::keychain::{Keychain, KeychainError, WalletSecret};
use cove_tokio::task::{self, spawn_actor};
use cove_util::result_ext::ResultExt as _;
use eyre::Context;
use flume::Sender;
use pubport::formats::Json;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, trace, warn};

/// Default number of addresses to scan
const DEFAULT_SCAN_LIMIT: u32 = 200;

use crate::{
    database::{
        Database,
        global_config::GlobalConfigTable,
        wallet_data::{ScanState, ScanningInfo, WalletDataDb},
    },
    manager::wallet_manager::{
        SingleOrMany, WalletManagerError, WalletManagerReconcileMessage, actor::WalletActor,
    },
    network::Network as CoveNetwork,
    node::{client::NodeClientOptions, client_builder::NodeClientBuilder},
    wallet::{
        WalletAddressType, WalletError,
        metadata::{DiscoveryState, FoundAddress, FoundJson, WalletId, WalletMetadata, WalletMode},
    },
    wallet_secret::WalletSecretExt as _,
};

#[derive(
    Debug, Default, derive_more::From, derive_more::Into, derive_more::Deref, derive_more::DerefMut,
)]
pub struct Wallets([Option<(WalletAddressType, BdkWallet)>; 2]);

#[derive(
    Debug, Default, derive_more::From, derive_more::Into, derive_more::Deref, derive_more::DerefMut,
)]
pub struct Workers([Option<WorkerHandle>; 2]);

#[derive(Debug)]
pub struct WorkerHandle {
    pub id: WalletId,
    pub addr: Addr<WalletDiscoveryWorker>,
    pub wallet_type: WalletAddressType,
    pub started_at: Instant,
    pub state: WorkerState,
    scan_task: Option<JoinHandle<()>>,
}

#[derive(Debug, Clone, Eq, PartialEq, Default)]
pub enum WorkerState {
    #[default]
    Created,
    Started,
    FoundAddress(String),
    NoneFound,
    Failed(String),
}

#[derive(Debug, Clone, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
pub enum WalletScannerError {
    #[error("No address types to scan")]
    NoAddressTypes,

    #[error("Unable to create wallet")]
    WalletCreationError(#[from] crate::wallet::WalletError),

    #[error("No wallet secret available for id {0}")]
    NoWalletSecretAvailable(WalletId),
}

#[derive(Debug, Clone, Eq, PartialEq, uniffi::Enum)]
pub enum ScannerResponse {
    FoundAddresses(Vec<FoundAddress>),
    NoneFound,
}

#[derive(Debug)]
pub(crate) struct WalletDiscoveryScanner {
    pub id: WalletId,
    pub addr: WeakAddr<Self>,
    pub workers: Workers,
    pub started_at: Instant,
    pub scan_source: ScanSource,
    pub responder: Sender<SingleOrMany>,
    pub wallet_actor: Addr<WalletActor>,
    failure_reconciled: bool,
    /// Cancels every scan task, including its in-flight node connection attempt.
    /// The scanner owns it so cancellation does not depend on worker actor health
    cancel: CancellationToken,
}

impl Drop for WalletDiscoveryScanner {
    fn drop(&mut self) {
        // a scanner that dies without a shutdown call must not leave scan tasks holding
        // the wallet database open
        self.cancel.cancel();
    }
}

#[derive(Debug, Clone)]
pub enum ScanSource {
    Json(Arc<FoundJson>),
    Mnemonic,
    Xprv,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct DiscoveryOrigin {
    network: CoveNetwork,
    wallet_mode: WalletMode,
}

impl From<&WalletMetadata> for DiscoveryOrigin {
    fn from(metadata: &WalletMetadata) -> Self {
        Self { network: metadata.network, wallet_mode: metadata.wallet_mode }
    }
}

#[derive(Debug, Clone, Copy)]
enum WalletSecretKind {
    Mnemonic,
    Xpriv,
}

impl WalletSecretKind {
    fn matches(self, secret: &WalletSecret) -> bool {
        matches!(
            (self, secret),
            (Self::Mnemonic, WalletSecret::Mnemonic(_)) | (Self::Xpriv, WalletSecret::Xpriv(_))
        )
    }
}

fn required_wallet_secret(
    result: Result<Option<WalletSecret>, KeychainError>,
    id: &WalletId,
    expected: WalletSecretKind,
) -> Result<WalletSecret, WalletScannerError> {
    let secret = result
        .map_err(WalletError::from)?
        .ok_or_else(|| WalletScannerError::NoWalletSecretAvailable(id.clone()))?;

    if !expected.matches(&secret) {
        return Err(WalletError::Keychain(KeychainError::WalletSecretTypeMismatch).into());
    }

    Ok(secret)
}

fn worker_failure_message(workers: &Workers) -> Option<String> {
    let failures = workers
        .iter()
        .filter_map(|worker| {
            let worker = worker.as_ref()?;
            let WorkerState::Failed(message) = &worker.state else {
                return None;
            };

            Some(format!("{:?}: {message}", worker.wallet_type))
        })
        .collect::<Vec<_>>();

    (!failures.is_empty()).then(|| failures.join("; "))
}

fn node_client_builder(
    global_config: &GlobalConfigTable,
    origin: DiscoveryOrigin,
) -> NodeClientBuilder {
    let node = global_config.selected_node_for_network(origin.network);
    let options = NodeClientOptions { batch_size: 1 };

    NodeClientBuilder { node, options }
}

#[async_trait::async_trait]
impl Actor for WalletDiscoveryScanner {
    async fn started(&mut self, addr: Addr<Self>) -> ActorResult<()> {
        self.addr = addr.downgrade();
        self.started_at = Instant::now();

        if let Err(error) = self.start_workers().await {
            let message = WalletManagerReconcileMessage::WalletError(
                WalletManagerError::WalletScanError(error.to_string()),
            );
            self.responder.send(message.into())?;
            self.failure_reconciled = true;
        }

        Produces::ok(())
    }

    async fn error(&mut self, error: ActorError) -> bool {
        error!("WalletDiscoveryScanner Error: {error:?}");
        false
    }
}

impl WalletDiscoveryScanner {
    /// Create a new scanner based on the DiscoveryState of the wallet metadata
    /// Only create a scanner if the discovery state is not already completed, and if
    /// have the required information to start a scan.
    pub(crate) fn try_new(
        metadata: WalletMetadata,
        wallet_actor: Addr<WalletActor>,
        reconciler: Sender<SingleOrMany>,
    ) -> Result<Self, WalletScannerError> {
        debug!("starting wallet discovery scanner for {}, metadata: {metadata:?}", metadata.id);

        let db = Database::global();
        let id = metadata.id.clone();
        let origin = DiscoveryOrigin::from(&metadata);
        let bdk_network = origin.network.into();
        let (wallets, scan_source) = match metadata.discovery_state {
            DiscoveryState::StartedJson(json) => (
                Wallets::try_from_json(&json, bdk_network, metadata.address_type)?,
                ScanSource::Json(json),
            ),
            DiscoveryState::StartedMnemonic => {
                let secret = required_wallet_secret(
                    Keychain::global().get_wallet_secret(&id),
                    &id,
                    WalletSecretKind::Mnemonic,
                )?;

                (Wallets::try_from_secret(secret, bdk_network)?, ScanSource::Mnemonic)
            }
            DiscoveryState::StartedXprv => {
                let secret = required_wallet_secret(
                    Keychain::global().get_wallet_secret(&id),
                    &id,
                    WalletSecretKind::Xpriv,
                )?;

                (Wallets::try_from_secret(secret, bdk_network)?, ScanSource::Xprv)
            }
            DiscoveryState::Single
            | DiscoveryState::NoneFound
            | DiscoveryState::ChoseAdressType
            | DiscoveryState::FoundAddressesFromJson(_, _)
            | DiscoveryState::FoundAddressesFromMnemonic(_)
            | DiscoveryState::FoundAddressesFromXprv(_) => {
                return Err(WalletScannerError::NoAddressTypes);
            }
        };

        if wallets.iter().all(Option::is_none) {
            return Err(WalletScannerError::NoAddressTypes);
        }

        let client_builder = node_client_builder(&db.global_config(), origin);
        Self::new(metadata.id, client_builder, wallets, scan_source, wallet_actor, reconciler)
    }

    fn new(
        id: WalletId,
        node_client_builder: NodeClientBuilder,
        wallets: Wallets,
        scan_source: ScanSource,
        wallet_actor: Addr<WalletActor>,
        reconciler: Sender<SingleOrMany>,
    ) -> Result<Self, WalletScannerError> {
        let mut started_workers = 0;
        let mut workers = Workers::default();

        // create workers
        for (wallet_type, wallet) in wallets.0.into_iter().flatten() {
            let worker = WalletDiscoveryWorker::try_new(
                id.clone(),
                wallet_type,
                wallet,
                node_client_builder.clone(),
            )?;

            let addr = spawn_actor(worker);
            workers[index(wallet_type)].replace(WorkerHandle {
                id: id.clone(),
                addr,
                wallet_type,
                started_at: Instant::now(),
                state: WorkerState::Created,
                scan_task: None,
            });

            started_workers += 1;
        }

        info!("started {started_workers} workers");

        Ok(Self {
            id,
            addr: Default::default(),
            workers,
            started_at: Instant::now(),
            scan_source,
            responder: reconciler,
            wallet_actor,
            failure_reconciled: false,
            cancel: CancellationToken::new(),
        })
    }

    async fn start_workers(&mut self) -> ActorResult<()> {
        let parent = self.addr.clone();
        let cancel = self.cancel.clone();

        for worker in self.workers.iter_mut().flatten() {
            let scan_task = call!(worker.addr.start(parent.clone(), cancel.clone())).await?;

            worker.scan_task = Some(scan_task);
            worker.state = WorkerState::Started;
        }

        self.finish_if_all_workers_stopped().await?;

        Produces::ok(())
    }

    pub async fn shutdown(&mut self) -> ActorResult<()> {
        debug!("shutting down wallet discovery scanner for {}", self.id);

        // stop every scan task before waiting on anything, so a task parked on a node
        // connection or on a callback into this scanner cannot hold shutdown open
        self.cancel.cancel();

        let mut scan_tasks = Vec::new();
        let mut worker_terminations = Vec::new();
        let mut shutdown_error = None;

        for worker_slot in &mut self.workers.0 {
            let Some(worker) = worker_slot.as_mut() else {
                continue;
            };

            if let Some(scan_task) = worker.scan_task.take() {
                scan_tasks.push(scan_task);
            }

            match call!(worker.addr.shutdown()).await {
                Ok(()) => {}
                Err(error) => {
                    shutdown_error.get_or_insert(error);
                }
            }

            worker_terminations.push(worker.addr.termination());
            *worker_slot = None;
        }

        // await only after every worker has received shutdown so callbacks cannot block progress
        for scan_task in scan_tasks {
            let _ = scan_task.await;
        }

        for worker_termination in worker_terminations {
            worker_termination.await;
        }

        if let Some(error) = shutdown_error {
            return Err(Box::new(error));
        }

        Produces::ok(())
    }

    pub async fn mark_found_txn(&mut self, wallet_type: WalletAddressType) -> ActorResult<()> {
        // shutdown clears the slot, so a late callback has no worker to mark
        let Some(worker) = self.workers[index(wallet_type)].as_mut() else {
            return Produces::ok(());
        };

        info!("marked worker {wallet_type:?} as found");

        let address = call!(worker.addr.first_address()).await?;
        worker.state = WorkerState::FoundAddress(address);

        self.finish_if_all_workers_stopped().await?;

        Produces::ok(())
    }

    pub async fn mark_limit_reached(&mut self, wallet_type: WalletAddressType) -> ActorResult<()> {
        let Some(worker) = self.workers[index(wallet_type)].as_mut() else {
            return Produces::ok(());
        };

        info!("marked worker {wallet_type:?} limit reached");

        worker.state = WorkerState::NoneFound;

        self.finish_if_all_workers_stopped().await?;

        Produces::ok(())
    }

    /// Records a worker failure and reconciles it after all workers stop
    pub async fn mark_failed(
        &mut self,
        wallet_type: WalletAddressType,
        message: String,
    ) -> ActorResult<()> {
        let Some(worker) = self.workers[index(wallet_type)].as_mut() else {
            return Produces::ok(());
        };

        error!("wallet discovery worker {wallet_type:?} failed: {message}");
        worker.state = WorkerState::Failed(message);

        self.finish_if_all_workers_stopped().await?;

        Produces::ok(())
    }

    async fn finish_if_all_workers_stopped(&mut self) -> ActorResult<()> {
        if self.failure_reconciled {
            return Produces::ok(());
        }

        if let Some(failure) = worker_failure_message(&self.workers) {
            let message = WalletManagerReconcileMessage::WalletError(
                WalletManagerError::WalletScanError(failure),
            );
            self.responder.send(message.into())?;
            self.failure_reconciled = true;

            return Produces::ok(());
        }

        let any_still_running = self.workers.iter().any(|worker| {
            worker.as_ref().is_some_and(|worker| {
                matches!(&worker.state, WorkerState::Created | WorkerState::Started)
            })
        });

        if any_still_running {
            return Produces::ok(());
        }

        let found_addresses = self.found_addresses();
        if found_addresses.is_empty() {
            self.set_metadata(DiscoveryState::NoneFound).await?;

            let message = WalletManagerReconcileMessage::from(ScannerResponse::NoneFound);
            self.responder.send(message.into())?;
        } else {
            self.set_metadata_address_found().await?;

            let message = WalletManagerReconcileMessage::from(ScannerResponse::FoundAddresses(
                found_addresses,
            ));
            self.responder.send(message.into())?;
        }

        Produces::ok(())
    }

    fn found_addresses(&self) -> Vec<FoundAddress> {
        self.workers
            .iter()
            .filter_map(|worker| {
                let worker = worker.as_ref()?;
                let WorkerState::FoundAddress(first_address) = &worker.state else {
                    return None;
                };

                Some(FoundAddress {
                    type_: worker.wallet_type,
                    first_address: first_address.clone(),
                })
            })
            .collect::<Vec<FoundAddress>>()
    }

    async fn set_metadata_address_found(&mut self) -> ActorResult<()> {
        match &self.scan_source {
            ScanSource::Json(json) => {
                self.set_metadata(DiscoveryState::FoundAddressesFromJson(
                    self.found_addresses(),
                    json.clone(),
                ))
                .await?;
            }

            ScanSource::Mnemonic => {
                self.set_metadata(DiscoveryState::FoundAddressesFromMnemonic(
                    self.found_addresses(),
                ))
                .await?;
            }

            ScanSource::Xprv => {
                self.set_metadata(DiscoveryState::FoundAddressesFromXprv(self.found_addresses()))
                    .await?;
            }
        }

        Produces::ok(())
    }

    async fn set_metadata(&mut self, discovery_state: DiscoveryState) -> ActorResult<()> {
        debug!("setting wallet metadata: {discovery_state:?}");
        call!(self.wallet_actor.update_discovery_state(discovery_state)).await??;

        Produces::ok(())
    }
}

// WORKER

#[derive(Debug)]
pub struct WalletDiscoveryWorker {
    parent: WeakAddr<WalletDiscoveryScanner>,
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
impl Actor for WalletDiscoveryWorker {
    async fn started(&mut self, addr: Addr<Self>) -> ActorResult<()> {
        self.addr = addr.downgrade();
        self.started_at = Instant::now();

        Produces::ok(())
    }

    async fn error(&mut self, error: ActorError) -> bool {
        error!("WalletDiscoveryWorker Error: {error:?}");
        false
    }
}

impl WalletDiscoveryWorker {
    /// Creates a discovery worker and restores its saved scan progress
    pub fn try_new(
        id: WalletId,
        wallet_type: WalletAddressType,
        wallet: BdkWallet,
        client_builder: NodeClientBuilder,
    ) -> Result<Self, WalletScannerError> {
        debug!("creating wallet discovery scanner for {id}, type: {wallet_type}");
        let db = WalletDataDb::new_or_existing(id.clone())
            .map_err(|error| WalletError::LoadError(error.to_string()))?;

        let scan_info = db
            .get_scan_state(wallet_type)
            .map_err(|error| WalletError::LoadError(error.to_string()))?
            .map(|scan_state| match scan_state {
                ScanState::Scanning(info) => info,
                ScanState::Completed => {
                    warn!("trying to scan completed wallet");
                    ScanningInfo::new(wallet_type)
                }
                _ => ScanningInfo::new(wallet_type),
            })
            .unwrap_or_else(|| ScanningInfo::new(wallet_type));

        debug!("wallet discovery scan info: {scan_info:?}");
        Ok(Self {
            parent: Default::default(),
            addr: Default::default(),
            wallet,
            client_builder,
            wallet_type,
            started_at: Instant::now(),
            scan_info,
            db,
            scan_limit: DEFAULT_SCAN_LIMIT,
        })
    }

    /// Spawns the scan task and hands its join handle to the scanner, which already owns
    /// `cancel` and can therefore stop the task even if this call never returns
    async fn start(
        &mut self,
        parent: WeakAddr<WalletDiscoveryScanner>,
        cancel: CancellationToken,
    ) -> ActorResult<JoinHandle<()>> {
        self.parent = parent;

        let wallet_type = self.wallet_type;
        let client_builder = self.client_builder.clone();
        let scan_limit = self.scan_limit;
        let current_address = self.scan_info.count;
        let parent = self.parent.clone();
        let failure_parent = parent.clone();
        let db = self.db.clone();
        let addr = self.addr.clone();

        let join = task::spawn(async move {
            let scan = async move {
                let run = async {
                    let client =
                        client_builder.build().await.context("build discovery node client")?;
                    let mut current_address = current_address;

                    loop {
                        let address = call!(addr.address_at(current_address)).await?;

                        let found_address = client
                            .check_address_for_txn(address)
                            .await
                            .context("could not check address")?;

                        if found_address {
                            db.set_scan_state(wallet_type, ScanState::Completed)
                                .context("save scan state")?;

                            call!(parent.mark_found_txn(wallet_type)).await?;
                            break;
                        }

                        current_address += 1;
                        trace!("checked {current_address} addresses for {wallet_type}");

                        // every 5 addresses, save the scan state
                        if current_address.is_multiple_of(5) {
                            let scan_state =
                                ScanningInfo { address_type: wallet_type, count: current_address };

                            if let Err(error) = db.set_scan_state(wallet_type, scan_state) {
                                error!("unable to update scan state: {error}");
                            }
                        }

                        if current_address >= scan_limit {
                            db.set_scan_state(wallet_type, ScanState::Completed)
                                .context("save completed scan state")?;

                            call!(parent.mark_limit_reached(wallet_type)).await?;
                            break;
                        }
                    }

                    Ok::<(), eyre::Error>(())
                };

                if let Err(error) = run.await {
                    error!("wallet scan failed: {error}");

                    if let Err(report_error) =
                        call!(failure_parent.mark_failed(wallet_type, error.to_string())).await
                    {
                        error!("unable to report wallet discovery scan failure: {report_error}");
                    }
                }
            };

            // covers the connection attempt and the failure report, so neither can outlive
            // a shutdown that is waiting on this task
            if cancel.run_until_cancelled(scan).await.is_none() {
                debug!("wallet discovery scan for {wallet_type} was canceled");
            }
        });

        Produces::ok(join)
    }

    pub async fn shutdown(&mut self) -> ActorResult<()> {
        self.parent = Default::default();

        Produces::ok(())
    }

    pub async fn first_address(&self) -> ActorResult<String> {
        let Produces::Value(address) = self.address_at(0).await? else {
            panic!("impossible");
        };

        Produces::ok(address.to_string())
    }

    async fn address_at(&self, index: u32) -> ActorResult<Address> {
        let address = self.wallet.peek_address(KeychainKind::External, index).address;

        Produces::ok(address)
    }
}

fn index(type_: WalletAddressType) -> usize {
    match type_ {
        WalletAddressType::WrappedSegwit => 0,
        WalletAddressType::Legacy => 1,
        WalletAddressType::NativeSegwit => panic!("NativeSegwit has no alternate discovery slot"),
    }
}

impl Wallets {
    pub fn try_from_json(
        json: &Json,
        network: Network,
        current_address_type: WalletAddressType,
    ) -> Result<Self, WalletError> {
        let mut wallets = Self::default();

        for (json, type_) in [
            (&json.bip49, WalletAddressType::WrappedSegwit),
            (&json.bip44, WalletAddressType::Legacy),
        ] {
            // skip the primary type, it's covered by the main wallet
            if type_ == current_address_type {
                continue;
            }

            if let Some(json) = json {
                // TODO: remove string round-trip once bdk_wallet updates to miniscript 13
                let external: ExtendedDescriptor =
                    json.external.to_string().parse().map_err_str(WalletError::BdkError)?;
                let internal: ExtendedDescriptor =
                    json.internal.to_string().parse().map_err_str(WalletError::BdkError)?;
                let params = BdkWallet::create(external, internal).network(network);

                let wallet =
                    BdkWallet::create_with_params(params).map_err_str(WalletError::BdkError)?;

                wallets[index(type_)] = Some((type_, wallet));
            }
        }

        Ok(wallets)
    }

    pub fn try_from_secret(secret: WalletSecret, network: Network) -> Result<Self, WalletError> {
        let mut wallets = Self::default();
        let cove_network = cove_types::Network::try_from(network).map_err(WalletError::BdkError)?;

        for type_ in [WalletAddressType::WrappedSegwit, WalletAddressType::Legacy] {
            let descriptor = secret.clone().into_descriptors(cove_network, type_);
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

impl From<ScannerResponse> for WalletManagerReconcileMessage {
    fn from(response: ScannerResponse) -> Self {
        Self::WalletScannerResponse(response)
    }
}

impl From<WalletScannerError> for WalletManagerReconcileMessage {
    fn from(error: WalletScannerError) -> Self {
        match error {
            WalletScannerError::NoAddressTypes => {
                Self::UnknownError("No alternate wallet address types to scan".to_string())
            }
            WalletScannerError::WalletCreationError(WalletError::Keychain(error)) => {
                Self::WalletError(WalletManagerError::SecretRetrievalError(error))
            }
            WalletScannerError::WalletCreationError(error) => {
                Self::WalletError(WalletManagerError::LoadWalletError(error))
            }
            WalletScannerError::NoWalletSecretAvailable(id) => Self::HotWalletKeyMissing(id),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use bdk_wallet::bitcoin::bip32::Xpriv;
    use cove_device::keychain::WalletXprv;

    use super::*;

    struct TaskCompletion(Arc<AtomicBool>);

    impl Drop for TaskCompletion {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    fn pubport_descriptors(descriptor: &str) -> pubport::descriptor::Descriptors {
        pubport::descriptor::Descriptors::try_from_line(descriptor)
            .expect("descriptor fixture is valid")
    }

    fn descriptor_json(
        bip44: Option<pubport::descriptor::Descriptors>,
        bip49: Option<pubport::descriptor::Descriptors>,
        bip84: Option<pubport::descriptor::Descriptors>,
    ) -> pubport::formats::Json {
        pubport::formats::Json { bip44, bip49, bip84, bip86: None }
    }

    fn test_global_config() -> (tempfile::TempDir, GlobalConfigTable) {
        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(redb::Database::create(tmp.path().join("test.redb")).unwrap());
        let write_txn = db.begin_write().unwrap();
        let global_config = GlobalConfigTable::new(db, &write_txn);
        write_txn.commit().unwrap();

        (tmp, global_config)
    }

    fn bip44_descriptors() -> pubport::descriptor::Descriptors {
        pubport_descriptors(
            "pkh([817e7be0/44h/0h/0h]xpub6BoKN14JzSFN1T3cqe9FnrwnXGAsmbgETJyeazoa3F7aMXh4XndvVrJAYyM127FsrH8KFv5XFXDroqXNfZMfsinow7xp93ueYSpnrjBBFs4/<0;1>/*)#tdtrl3y9",
        )
    }

    fn bip49_descriptors() -> pubport::descriptor::Descriptors {
        pubport_descriptors(
            "sh(wpkh([817e7be0/49h/0h/0h]xpub6CCKAvUTNursEnaJ8k1d27LfqEUzeAx2N9wFqYE3W1xh7nqgJEBEbLSSmohwDxzsSvcsYqiQqFzRvta65Njbe5o84bF5YXHFqfSH2Dkhonm/<0;1>/*))#8llmt36x",
        )
    }

    fn bip84_descriptors() -> pubport::descriptor::Descriptors {
        pubport_descriptors(
            "wpkh([817e7be0/84h/0h/0h]xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM/<0;1>/*)#60tjs4c7",
        )
    }

    #[test]
    fn json_discovery_builds_wrapped_and_legacy_alternates_for_native_segwit() {
        let json = descriptor_json(
            Some(bip44_descriptors()),
            Some(bip49_descriptors()),
            Some(bip84_descriptors()),
        );

        let wallets =
            Wallets::try_from_json(&json, Network::Bitcoin, WalletAddressType::NativeSegwit)
                .unwrap();

        assert!(wallets[index(WalletAddressType::WrappedSegwit)].is_some());
        assert!(wallets[index(WalletAddressType::Legacy)].is_some());
    }

    #[test]
    fn json_discovery_skips_current_wrapped_segwit_alternate() {
        let json = descriptor_json(
            Some(bip44_descriptors()),
            Some(bip49_descriptors()),
            Some(bip84_descriptors()),
        );

        let wallets =
            Wallets::try_from_json(&json, Network::Bitcoin, WalletAddressType::WrappedSegwit)
                .unwrap();

        assert!(wallets[index(WalletAddressType::WrappedSegwit)].is_none());
        assert!(wallets[index(WalletAddressType::Legacy)].is_some());
    }

    #[test]
    fn xprv_discovery_builds_wrapped_and_legacy_alternates() {
        let xprv = Xpriv::new_master(Network::Bitcoin, &[23; 32]).unwrap();
        let secret = WalletSecret::Xpriv(WalletXprv::try_from(xprv).unwrap());

        let wallets = Wallets::try_from_secret(secret, Network::Bitcoin).unwrap();

        assert!(wallets[index(WalletAddressType::WrappedSegwit)].is_some());
        assert!(wallets[index(WalletAddressType::Legacy)].is_some());
    }

    #[test]
    fn discovery_origin_comes_from_wallet_metadata() {
        let mut metadata = WalletMetadata::preview_new();
        metadata.network = CoveNetwork::Signet;
        metadata.wallet_mode = WalletMode::Decoy;

        let origin = DiscoveryOrigin::from(&metadata);

        assert_eq!(
            origin,
            DiscoveryOrigin { network: CoveNetwork::Signet, wallet_mode: WalletMode::Decoy }
        );
    }

    #[test]
    fn discovery_uses_origin_network_node_when_global_network_differs() {
        crate::app::reconcile::test_support::init_noop_updater();
        let (_tmp, global_config) = test_global_config();
        let signet_node = crate::node::Node::new_esplora(
            "Signet fixture".into(),
            "https://signet.example/api".into(),
            CoveNetwork::Signet,
        );
        global_config.set_selected_node(&signet_node).unwrap();
        global_config.set_selected_network(CoveNetwork::Bitcoin).unwrap();

        let origin =
            DiscoveryOrigin { network: CoveNetwork::Signet, wallet_mode: WalletMode::Main };
        let client_builder = node_client_builder(&global_config, origin);

        assert_eq!(global_config.selected_network(), CoveNetwork::Bitcoin);
        assert_eq!(client_builder.node, signet_node);
    }

    #[test]
    fn wallet_secret_keychain_error_is_preserved() {
        let id = WalletId::preview_new_random();
        let keychain_error = KeychainError::Decrypt("fixture failure".to_string());

        let error =
            required_wallet_secret(Err(keychain_error.clone()), &id, WalletSecretKind::Mnemonic)
                .unwrap_err();

        assert!(matches!(
            error,
            WalletScannerError::WalletCreationError(WalletError::Keychain(error))
                if error == keychain_error
        ));
    }

    #[test]
    fn wallet_secret_type_mismatch_is_not_reported_as_missing() {
        let id = WalletId::preview_new_random();
        let xprv = Xpriv::new_master(Network::Bitcoin, &[42; 32]).unwrap();
        let secret = WalletSecret::Xpriv(WalletXprv::try_from(xprv).unwrap());

        let error =
            required_wallet_secret(Ok(Some(secret)), &id, WalletSecretKind::Mnemonic).unwrap_err();

        assert!(matches!(
            error,
            WalletScannerError::WalletCreationError(WalletError::Keychain(
                KeychainError::WalletSecretTypeMismatch
            ))
        ));
    }

    #[test]
    fn scanner_setup_failure_maps_to_typed_wallet_reconcile_error() {
        let keychain_error = KeychainError::Decrypt("fixture failure".into());
        let message = WalletManagerReconcileMessage::from(WalletScannerError::WalletCreationError(
            WalletError::Keychain(keychain_error.clone()),
        ));

        assert_eq!(
            message,
            WalletManagerReconcileMessage::WalletError(WalletManagerError::SecretRetrievalError(
                keychain_error
            ))
        );
    }

    #[test]
    fn worker_failure_is_ready_to_reconcile_while_another_worker_is_running() {
        let id = WalletId::preview_new_random();
        let mut workers = Workers::default();
        workers[index(WalletAddressType::WrappedSegwit)] = Some(WorkerHandle {
            id: id.clone(),
            addr: Addr::default(),
            wallet_type: WalletAddressType::WrappedSegwit,
            started_at: Instant::now(),
            state: WorkerState::Failed("node unavailable".into()),
            scan_task: None,
        });

        workers[index(WalletAddressType::Legacy)] = Some(WorkerHandle {
            id,
            addr: Addr::default(),
            wallet_type: WalletAddressType::Legacy,
            started_at: Instant::now(),
            state: WorkerState::Started,
            scan_task: None,
        });

        assert_eq!(
            worker_failure_message(&workers),
            Some("WrappedSegwit: node unavailable".into())
        );
    }

    #[tokio::test]
    async fn scanner_shutdown_cancels_and_awaits_worker_task() {
        crate::test_support::ensure_tokio_runtime();

        let id = WalletId::preview_new_random();
        let json = descriptor_json(None, Some(bip49_descriptors()), None);
        let (wallet_type, wallet) =
            Wallets::try_from_json(&json, Network::Bitcoin, WalletAddressType::NativeSegwit)
                .expect("discovery wallet is created")
                .0
                .into_iter()
                .flatten()
                .next()
                .expect("discovery worker exists");
        let db = WalletDataDb::new_in_memory(id.clone()).expect("worker database is created");
        let db_ref = Arc::clone(&db.db);
        let task_finished = Arc::new(AtomicBool::new(false));
        let (task_started, task_started_seen) = tokio::sync::oneshot::channel();

        // stands in for the real scan task: it holds the wallet database open until the
        // scanner cancels it
        let cancel = CancellationToken::new();
        let cancel_for_task = cancel.clone();
        let db_for_task = Arc::clone(&db_ref);
        let task_finished_for_task = Arc::clone(&task_finished);
        let scan_task = cove_tokio::task::spawn(async move {
            let _task_completion = TaskCompletion(task_finished_for_task);
            let _db = db_for_task;

            task_started.send(()).expect("task observer is waiting");
            cancel_for_task.run_until_cancelled(std::future::pending::<()>()).await;
        });
        task_started_seen.await.expect("worker task starts");

        let worker = WalletDiscoveryWorker {
            parent: WeakAddr::default(),
            addr: WeakAddr::default(),
            client_builder: NodeClientBuilder {
                node: crate::node::Node::new_esplora(
                    "scanner fixture".into(),
                    "http://127.0.0.1".into(),
                    CoveNetwork::Bitcoin,
                ),
                options: NodeClientOptions { batch_size: 1 },
            },
            wallet_type,
            wallet,
            started_at: Instant::now(),
            scan_info: ScanningInfo::new(wallet_type),
            db,
            scan_limit: DEFAULT_SCAN_LIMIT,
        };
        let worker_addr = spawn_actor(worker);
        let worker_termination = worker_addr.termination();
        let (responder, _responses) = flume::bounded(1);
        let scanner = spawn_actor(WalletDiscoveryScanner {
            id: id.clone(),
            addr: WeakAddr::default(),
            workers: Workers([
                Some(WorkerHandle {
                    id,
                    addr: worker_addr.clone(),
                    wallet_type,
                    started_at: Instant::now(),
                    state: WorkerState::Started,
                    scan_task: Some(scan_task),
                }),
                None,
            ]),
            started_at: Instant::now(),
            scan_source: ScanSource::Mnemonic,
            responder,
            wallet_actor: Addr::detached(),
            failure_reconciled: false,
            cancel,
        });
        drop(worker_addr);

        tokio::time::timeout(std::time::Duration::from_secs(1), call!(scanner.shutdown()))
            .await
            .expect("scanner shutdown waits for the worker task")
            .expect("scanner shutdown responds");

        assert!(task_finished.load(Ordering::SeqCst));
        assert_eq!(Arc::strong_count(&db_ref), 1);

        tokio::time::timeout(std::time::Duration::from_secs(1), worker_termination)
            .await
            .expect("worker actor terminates");
        assert_eq!(Arc::strong_count(&db_ref), 1);
    }
}
