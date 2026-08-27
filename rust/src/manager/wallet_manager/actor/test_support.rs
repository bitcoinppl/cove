use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use act_zero::{ActorResult, Addr, runtimes::tokio::spawn_actor};
use bitcoin::{Transaction, absolute::LockTime, transaction::Version};
use cove_device::keychain::Keychain;
use cove_types::network::Network as CoveNetwork;
use parking_lot::RwLock;
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpListener,
    task::JoinHandle,
};

use crate::{
    database::wallet_data::WalletDataDb,
    manager::wallet_manager::{WalletScanStatus, WalletSnapshot},
    node::Node,
    wallet::Wallet,
};

use super::{SingleOrMany, WalletActor};

impl WalletActor {
    pub(crate) fn new_with_db(
        wallet: Wallet,
        reconciler: flume::Sender<SingleOrMany>,
        scan_status: Arc<RwLock<WalletScanStatus>>,
        wallet_snapshot: Arc<RwLock<WalletSnapshot>>,
        db: WalletDataDb,
    ) -> Self {
        let metadata = Arc::new(RwLock::new(wallet.metadata.clone()));

        Self::new_with_metadata_and_db(
            wallet,
            reconciler,
            scan_status,
            wallet_snapshot,
            db,
            metadata,
        )
    }
}

pub(crate) struct BroadcastEsploraNode {
    pub(crate) broadcast_requests: Arc<AtomicUsize>,
    pub(crate) server: JoinHandle<()>,
}

pub(crate) struct PendingBroadcastEsploraNode {
    pub(crate) broadcast_requests: Arc<AtomicUsize>,
    pub(crate) release: tokio::sync::watch::Sender<bool>,
    pub(crate) server: JoinHandle<()>,
}

pub(crate) async fn actor_value<T>(result: ActorResult<T>) -> T {
    result
        .expect("actor method should not fail")
        .await
        .expect("actor method should produce a value")
}

pub(crate) fn test_broadcast_transaction() -> Transaction {
    Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: Vec::new(),
        output: Vec::new(),
    }
}

pub(crate) fn new_test_wallet_actor(
    wallet: Wallet,
    sender: flume::Sender<SingleOrMany>,
) -> WalletActor {
    crate::test_support::ensure_tokio_runtime();

    let wallet_snapshot = Arc::new(RwLock::new(WalletSnapshot::from_wallet(&wallet)));
    let scan_status = Arc::new(RwLock::new(WalletScanStatus::Idle));
    let metadata = Arc::new(RwLock::new(wallet.metadata.clone()));

    WalletActor::new_with_metadata(wallet, sender, scan_status, wallet_snapshot, metadata)
        .expect("actor is created")
}

pub(crate) fn new_test_wallet_actor_with_db(
    wallet: Wallet,
    sender: flume::Sender<SingleOrMany>,
    db: WalletDataDb,
) -> WalletActor {
    crate::test_support::ensure_tokio_runtime();

    let wallet_snapshot = Arc::new(RwLock::new(WalletSnapshot::from_wallet(&wallet)));
    let scan_status = Arc::new(RwLock::new(WalletScanStatus::Idle));

    WalletActor::new_with_db(wallet, sender, scan_status, wallet_snapshot, db)
}

pub(crate) fn test_keychain() -> &'static Keychain {
    crate::test_support::init_test_keychain();
    Keychain::global()
}

pub(crate) fn spawn_test_wallet_actor(
    wallet: Wallet,
) -> (Addr<WalletActor>, flume::Receiver<SingleOrMany>) {
    let (sender, receiver) = flume::bounded(100);
    let actor = new_test_wallet_actor(wallet, sender);
    let addr = spawn_actor(actor);

    (addr, receiver)
}

pub(crate) async fn set_broadcast_esplora_node(broadcast_status: u16) -> BroadcastEsploraNode {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("test esplora server binds");
    let address = listener.local_addr().expect("test esplora server has address");
    let node = Node::new_esplora(
        "broadcast test esplora node".to_string(),
        format!("http://{address}"),
        CoveNetwork::Bitcoin,
    );

    crate::database::Database::global()
        .global_config
        .set_selected_node(&node)
        .expect("test node config is saved");

    let broadcast_requests = Arc::new(AtomicUsize::new(0));
    let broadcast_counter = broadcast_requests.clone();
    let server = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else { return };
            let broadcast_counter = broadcast_counter.clone();

            tokio::spawn(async move {
                let mut request = [0; 8192];
                let bytes_read = stream.read(&mut request).await.unwrap_or_default();
                let request = String::from_utf8_lossy(&request[..bytes_read]);

                let body = if request.starts_with("POST /tx ") {
                    broadcast_counter.fetch_add(1, Ordering::SeqCst);
                    "broadcast"
                } else {
                    "1"
                };

                let status = if request.starts_with("POST /tx ") { broadcast_status } else { 200 };
                let reason = if status == 200 { "OK" } else { "Internal Server Error" };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });

    BroadcastEsploraNode { broadcast_requests, server }
}

pub(crate) async fn set_pending_broadcast_esplora_node() -> PendingBroadcastEsploraNode {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("test esplora server binds");
    let address = listener.local_addr().expect("test esplora server has address");
    let node = Node::new_esplora(
        "pending broadcast test esplora node".to_string(),
        format!("http://{address}"),
        CoveNetwork::Bitcoin,
    );

    crate::database::Database::global()
        .global_config
        .set_selected_node(&node)
        .expect("test node config is saved");

    let broadcast_requests = Arc::new(AtomicUsize::new(0));
    let broadcast_counter = broadcast_requests.clone();
    let (release, release_request) = tokio::sync::watch::channel(false);
    let server = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else { return };
            let broadcast_counter = broadcast_counter.clone();
            let release_request = release_request.clone();

            tokio::spawn(async move {
                let mut request = [0; 8192];
                let bytes_read = stream.read(&mut request).await.unwrap_or_default();
                let request = String::from_utf8_lossy(&request[..bytes_read]);
                let is_broadcast = request.starts_with("POST /tx ");
                if is_broadcast {
                    broadcast_counter.fetch_add(1, Ordering::SeqCst);
                    let mut release_request = release_request.clone();

                    while !*release_request.borrow() {
                        if release_request.changed().await.is_err() {
                            return;
                        }
                    }
                }

                let body = if is_broadcast { "broadcast" } else { "1" };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });

    PendingBroadcastEsploraNode { broadcast_requests, release, server }
}

pub(crate) async fn wait_for_broadcast_request_count(
    broadcast_requests: &Arc<AtomicUsize>,
    count: usize,
) {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if broadcast_requests.load(Ordering::SeqCst) >= count {
                return;
            }

            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("broadcast request count is reached");
}

pub(crate) fn restore_default_bitcoin_node() {
    let node = Node::default(CoveNetwork::Bitcoin);

    crate::database::Database::global()
        .global_config
        .set_selected_node(&node)
        .expect("default node config is saved");
}

pub(crate) fn mark_wallet_ledger_ready(wallet: &mut Wallet) {
    wallet.metadata.internal.performed_full_scan_at = Some(1);
}
