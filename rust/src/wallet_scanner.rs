use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use act_zero::*;
use bdk_wallet::{KeychainKind, Wallet as BdkWallet};
use tokio::sync::mpsc::Sender;
use tracing::{debug, error, info};

use crate::{node::client::NodeClient, task::spawn_actor};

const DEFAULT_SCAN_TIMEOUT: u8 = 30;

#[derive(
    Debug, Default, derive_more::From, derive_more::Into, derive_more::Deref, derive_more::DerefMut,
)]
pub struct Wallets([Option<(WalletAddressType, BdkWallet)>; 3]);

#[derive(
    Debug,
    Clone,
    Eq,
    PartialEq,
    Default,
    derive_more::From,
    derive_more::Into,
    derive_more::Deref,
    derive_more::DerefMut,
)]
pub struct Workers([Option<Worker>; 3]);

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Worker {
    pub addr: Addr<WalletScanWorker>,
    pub wallet_type: WalletAddressType,
    pub started_at: Instant,
    pub state: WorkerState,
}

#[derive(Debug, Clone, Eq, PartialEq, Copy, Default)]
pub enum WorkerState {
    #[default]
    Created,
    Started,
    FoundAddress,
    NoneFound,
}

#[derive(Debug, Clone, Eq, PartialEq, Copy)]
pub enum WalletAddressType {
    Segwit,
    WrappedSegwit,
    Legacy,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ScannerResponse {
    TimeoutExpired(Vec<WalletAddressType>),
    FoundAddresses(Vec<WalletAddressType>),
    NoneFound,
}

#[derive(Debug, Clone)]
pub struct WalletScanner {
    pub addr: WeakAddr<Self>,
    pub workers: Workers,
    pub started_at: Instant,
    pub node_client: Arc<NodeClient>,

    responder: Sender<ScannerResponse>,
    // in seconds
    scan_timeout: u8,
}

#[async_trait::async_trait]
impl Actor for WalletScanner {
    async fn started(&mut self, addr: Addr<Self>) -> ActorResult<()> {
        self.addr = addr.downgrade();
        self.started_at = Instant::now();

        let timeout = self.scan_timeout.into();

        // run timeout_expired function after timeout
        let addr_clone = self.addr.clone();
        addr.send_fut(async move {
            tokio::time::sleep(Duration::from_secs(timeout)).await;

            if let Err(error) = call!(addr_clone.timeout_expired()).await {
                error!("timeout expired error: {error}");
            }
        });

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
    pub fn new(
        node_client: impl Into<Arc<NodeClient>>,
        wallets: [Option<(WalletAddressType, BdkWallet)>; 3],
        responder: Sender<ScannerResponse>,
    ) -> Self {
        let node_client = node_client.into();
        let mut started_workers = 0;
        let mut workers = Workers::default();

        // create workers
        for (wallet_type, wallet) in wallets.into_iter().flatten() {
            let worker = WalletScanWorker::new(wallet_type, wallet, node_client.clone());

            let addr = spawn_actor(worker);
            workers[wallet_type.index()].replace(Worker {
                addr,
                wallet_type,
                started_at: Instant::now(),
                state: WorkerState::Created,
            });

            started_workers += 1;
        }

        info!("started {started_workers} workers");

        Self {
            addr: Default::default(),
            workers,
            started_at: Instant::now(),
            node_client,
            responder,
            scan_timeout: DEFAULT_SCAN_TIMEOUT,
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
        debug!("marked worker {wallet_type:?} as found");

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
                .send(ScannerResponse::FoundAddresses(found_addresses))
                .await?;

            return Produces::ok(());
        }

        // some workers are still running, check if timeout has expired
        if self.started_at.elapsed().as_secs() > self.scan_timeout as u64 {
            self.responder.send(ScannerResponse::NoneFound).await?;
        }

        Produces::ok(())
    }

    // timeout expired, check and send response
    pub async fn timeout_expired(&mut self) -> ActorResult<()> {
        debug!("timeout expired");

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
            self.responder.send(ScannerResponse::NoneFound).await?;
            return Produces::ok(());
        }

        self.responder
            .send(ScannerResponse::TimeoutExpired(found_addresses))
            .await?;

        Produces::ok(())
    }
}

// WORKER

#[derive(Debug)]
pub struct WalletScanWorker {
    parent: WeakAddr<WalletScanner>,
    addr: WeakAddr<Self>,
    client: Arc<NodeClient>,
    wallet_type: WalletAddressType,
    wallet: BdkWallet,
    started_at: Instant,
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
    pub fn new(wallet_type: WalletAddressType, wallet: BdkWallet, client: Arc<NodeClient>) -> Self {
        Self {
            parent: Default::default(),
            addr: Default::default(),
            wallet,
            client,
            wallet_type,
            started_at: Instant::now(),
        }
    }

    pub async fn start(&mut self, parent: WeakAddr<WalletScanner>) -> ActorResult<()> {
        self.parent = parent;
        self.start_scan().await
    }

    async fn start_scan(&mut self) -> ActorResult<()> {
        let mut addresses_checked = 0;

        loop {
            let address = self.wallet.reveal_next_address(KeychainKind::External);

            // found address
            if self.client.check_address_for_txn(address.address).await? {
                call!(self.parent.mark_found_txn(self.wallet_type));
                return Produces::ok(());
            }

            addresses_checked += 1;
            debug!(
                "checked {addresses_checked} addresses for {:?}",
                self.wallet_type
            );
        }
    }
}

impl WalletAddressType {
    pub fn index(&self) -> usize {
        match self {
            WalletAddressType::Segwit => 0_usize,
            WalletAddressType::WrappedSegwit => 1,
            WalletAddressType::Legacy => 2,
        }
    }
}
