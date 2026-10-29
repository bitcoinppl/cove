use std::time::Duration;

use act_zero::{runtimes::tokio::spawn_actor, *};
use bdk_wallet::KeychainKind;
use bitcoin::{Amount, Network, ScriptBuf};

use super::{
    SingleOrMany, WalletManagerReconcileMessage, drain_reconcile_messages, new_test_wallet_actor,
    one_sat_vbyte_fee_rate, persisted_preview_wallet, restore_default_bitcoin_node,
};
use crate::{
    manager::wallet_manager::{
        WalletScanPhase, WalletScanStatus,
        ledger_state::{InitialScanActivity, WalletLedgerState},
    },
    network::Network as CoveNetwork,
    node::Node,
    test_support::fake_node::{FakeNode, Protocol, wait_until},
    transaction::Transaction,
    wallet::{
        Address,
        metadata::{WalletId, WalletMetadata},
    },
};

type Msg = WalletManagerReconcileMessage;

struct Fixture {
    node: FakeNode,
    addr: Addr<super::super::WalletActor>,
    observed: Observed,
    wallet_id: WalletId,
    first_receive: ScriptBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        restore_default_bitcoin_node();
        restore_default_signet_node();
        restore_selected_network();
        let _ = crate::wallet::delete_wallet_specific_data(&self.wallet_id);
    }
}

struct Observed {
    receiver: flume::Receiver<SingleOrMany>,
    seen: Vec<Msg>,
}

fn restore_default_signet_node() {
    crate::database::Database::global()
        .global_config
        .set_selected_node(&Node::default(CoveNetwork::Signet))
        .expect("default signet node config is saved");
}

fn restore_selected_network() {
    crate::database::Database::global()
        .global_config
        .set_selected_network(CoveNetwork::Bitcoin)
        .expect("selected network is restored");
}

fn fixture() -> Fixture {
    fixture_on(Network::Bitcoin)
}

fn fixture_on(network: Network) -> Fixture {
    crate::database::test_support::init_test_database();

    let node = FakeNode::start(network);
    let mut metadata = WalletMetadata::preview_new();
    metadata.network = match network {
        Network::Signet => CoveNetwork::Signet,
        _ => CoveNetwork::Bitcoin,
    };

    // the actor connects to the node of the globally selected network
    crate::database::Database::global()
        .global_config
        .set_selected_network(metadata.network)
        .expect("selected network is set");
    let wallet = persisted_preview_wallet(metadata.clone());
    let first_receive = wallet.bdk.peek_address(KeychainKind::External, 0).script_pubkey();

    let (sender, receiver) = flume::unbounded();
    let addr = spawn_actor(new_test_wallet_actor(wallet, sender));
    drain_reconcile_messages(&receiver);

    Fixture {
        node,
        addr,
        observed: Observed { receiver, seen: Vec::new() },
        wallet_id: metadata.id,
        first_receive,
    }
}

impl Observed {
    fn absorb(&mut self, batch: SingleOrMany) {
        match batch {
            SingleOrMany::Single(message) => self.seen.push(message),
            SingleOrMany::Many(messages) => self.seen.extend(messages),
        }
    }

    async fn until(&mut self, what: &str, done: impl Fn(&Msg) -> bool) {
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                if self.seen.iter().any(&done) {
                    return;
                }

                let batch = self.receiver.recv_async().await.expect("reconciler stays open");
                self.absorb(batch);
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {what}; saw {:#?}", self.seen));
    }

    /// Pulls in everything the actor has already reconciled
    fn settle(&mut self) {
        while let Ok(batch) = self.receiver.try_recv() {
            self.absorb(batch);
        }
    }

    fn count(&self, matches: impl Fn(&Msg) -> bool) -> usize {
        self.seen.iter().filter(|message| matches(message)).count()
    }

    fn ledger_states(&self) -> Vec<WalletLedgerState> {
        self.seen
            .iter()
            .filter_map(|message| match message {
                Msg::LedgerStateChanged(state) => Some(*state),
                _ => None,
            })
            .collect()
    }

    fn scan_statuses(&self) -> Vec<WalletScanStatus> {
        self.seen
            .iter()
            .filter_map(|message| match message {
                Msg::WalletScanStatusChanged(status) => Some(status.clone()),
                _ => None,
            })
            .collect()
    }

    fn last_balance_sats(&self) -> Option<u64> {
        self.seen.iter().rev().find_map(|message| match message {
            Msg::WalletBalanceChanged(balance) => Some(balance.confirmed.to_sat()),
            _ => None,
        })
    }
}

fn is_scan_complete(message: &Msg) -> bool {
    matches!(message, Msg::ScanComplete(_))
}

fn is_idle(message: &Msg) -> bool {
    matches!(message, Msg::WalletScanStatusChanged(WalletScanStatus::Idle))
}

fn scripthash_history_requests(node: &FakeNode) -> Vec<String> {
    node.requests()
        .into_iter()
        .filter(|request| match request.protocol {
            Protocol::Esplora => request.name.starts_with("/scripthash/"),
            Protocol::Electrum => request.name == "blockchain.scripthash.get_history",
        })
        .map(|request| format!("{}{}", request.name, request.detail))
        .collect()
}

/// Every Full-phase scan progress report in the order it was emitted
fn full_scan_basis_points(statuses: &[WalletScanStatus]) -> Vec<u32> {
    statuses
        .iter()
        .filter_map(|status| match status {
            WalletScanStatus::Scanning(progress) if progress.phase == WalletScanPhase::Full => {
                Some(progress.progress_basis_points)
            }
            _ => None,
        })
        .collect()
}

fn scan_complete_transactions(observed: &Observed) -> Vec<Transaction> {
    observed
        .seen
        .iter()
        .find_map(|message| match message {
            Msg::ScanComplete(transactions) => Some(transactions.clone()),
            _ => None,
        })
        .expect("scan completed")
}

#[tokio::test(flavor = "current_thread")]
async fn unused_wallet_initial_scan_reports_progress_then_completes() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let mut fx = fixture();
    fx.node.select_esplora();

    // hold the first address lookups so the mid-scan state can be observed
    let (gate, _rule) = fx.node.on("/scripthash/").hold();

    send!(fx.addr.wallet_scan_and_notify(false));
    wait_until("the scan to reach the node", || gate.waiting() > 0).await;
    fx.observed
        .until("the incomplete ledger to be reported", |message| {
            matches!(
                message,
                Msg::LedgerStateChanged(WalletLedgerState::InitialScanIncomplete(
                    InitialScanActivity::Active
                ))
            )
        })
        .await;
    fx.observed.settle();

    assert_eq!(
        fx.observed.count(is_scan_complete),
        0,
        "the scan must not report completion while the node has not answered"
    );
    assert!(
        !fx.observed.ledger_states().contains(&WalletLedgerState::Complete),
        "the ledger must stay incomplete until the first scan finishes"
    );
    assert!(
        !fx.observed.seen.iter().any(|message| matches!(
            message,
            Msg::WalletMetadataChanged(metadata) if metadata.internal.performed_full_scan_at.is_some()
        )),
        "the first full scan must not be recorded before it finishes"
    );

    gate.open();
    fx.observed.until("the scan to complete", is_scan_complete).await;
    fx.observed.until("the scan status to go idle", is_idle).await;
    fx.observed.settle();

    let basis_points = full_scan_basis_points(&fx.observed.scan_statuses());
    assert!(!basis_points.is_empty(), "a full scan must report progress");
    assert!(
        basis_points.windows(2).all(|pair| pair[0] <= pair[1]),
        "progress must never go backwards, got {basis_points:?}"
    );
    assert!(
        basis_points.last().copied().unwrap_or_default() > 0,
        "progress must advance past zero, got {basis_points:?}"
    );

    assert!(
        scan_complete_transactions(&fx.observed).is_empty(),
        "an unused wallet has no transactions"
    );
    assert_eq!(fx.observed.last_balance_sats(), Some(0));
    assert_eq!(fx.observed.ledger_states().last(), Some(&WalletLedgerState::Complete));
    assert!(
        fx.observed.seen.iter().any(|message| matches!(
            message,
            Msg::WalletMetadataChanged(metadata) if metadata.internal.performed_full_scan_at.is_some()
        )),
        "completing the first scan must be recorded in the wallet metadata"
    );
    assert_eq!(
        fx.observed.scan_statuses().last(),
        Some(&WalletScanStatus::Idle),
        "the scan status must settle back to idle"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn unused_signet_wallet_initial_scan_completes() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let mut fx = fixture_on(Network::Signet);
    fx.node.select_esplora();

    send!(fx.addr.wallet_scan_and_notify(false));
    fx.observed.until("the scan to complete", is_scan_complete).await;
    fx.observed.until("the scan status to go idle", is_idle).await;
    fx.observed.settle();

    assert!(scan_complete_transactions(&fx.observed).is_empty());
    assert_eq!(fx.observed.last_balance_sats(), Some(0));
    assert_eq!(
        fx.observed.ledger_states().first(),
        Some(&WalletLedgerState::InitialScanIncomplete(InitialScanActivity::Active)),
        "a signet wallet starts out incomplete"
    );
    assert_eq!(fx.observed.ledger_states().last(), Some(&WalletLedgerState::Complete));
    assert_eq!(fx.observed.scan_statuses().last(), Some(&WalletScanStatus::Idle));
    assert!(
        fx.observed.seen.iter().any(|message| matches!(
            message,
            Msg::WalletMetadataChanged(metadata) if metadata.internal.performed_full_scan_at.is_some()
        )),
        "completing the first scan must be recorded in the wallet metadata"
    );
    assert!(
        scripthash_history_requests(&fx.node).len() > 1,
        "the scan must have asked the signet node about the wallet's addresses"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn repeated_scan_requests_during_the_first_scan_run_it_once() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let mut fx = fixture();
    fx.node.select_esplora();

    let (gate, _rule) = fx.node.on("/scripthash/").hold();

    // the screen can ask for a scan on appear, on foreground and on refresh
    send!(fx.addr.wallet_scan_and_notify(false));
    send!(fx.addr.wallet_scan_and_notify(false));
    send!(fx.addr.wallet_scan_and_notify(false));

    wait_until("the scan to reach the node", || gate.waiting() > 0).await;
    gate.open();

    fx.observed.until("the scan to complete", is_scan_complete).await;
    fx.observed.until("the scan status to go idle", is_idle).await;

    // give any duplicate scan time to surface before counting
    tokio::time::sleep(Duration::from_millis(500)).await;
    fx.observed.settle();

    assert_eq!(fx.observed.count(is_scan_complete), 1, "the initial scan must complete once");

    let lookups = scripthash_history_requests(&fx.node);
    let unique = lookups.iter().collect::<std::collections::HashSet<_>>();
    assert_eq!(
        lookups.len(),
        unique.len(),
        "no address may be looked up twice by overlapping scans"
    );

    // a request right after the scan finished is inside the recent-scan window
    let lookups_before = lookups.len();
    call!(fx.addr.wallet_scan_and_notify(false)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    fx.observed.settle();

    assert_eq!(
        scripthash_history_requests(&fx.node).len(),
        lookups_before,
        "a scan requested right after a finished scan must be skipped"
    );
    assert_eq!(fx.observed.count(is_scan_complete), 1);
    assert_eq!(fx.observed.scan_statuses().last(), Some(&WalletScanStatus::Idle));
}

#[tokio::test(flavor = "current_thread")]
async fn scan_reports_confirmed_funds_found_on_chain() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let mut fx = fixture();
    fx.node.chain().mine(5);
    let txid = fx.node.chain().pay(&fx.first_receive, Amount::from_sat(50_000));
    let height = fx.node.chain().confirm(txid);
    fx.node.select_esplora();

    send!(fx.addr.wallet_scan_and_notify(false));
    fx.observed.until("the scan to complete", is_scan_complete).await;
    fx.observed.until("the scan status to go idle", is_idle).await;
    fx.observed.settle();

    let transactions = scan_complete_transactions(&fx.observed);
    assert_eq!(transactions.len(), 1, "the funding transaction is the wallet's only transaction");
    assert!(
        matches!(&transactions[0], Transaction::Confirmed(confirmed) if confirmed.id().0 == txid),
        "the funding transaction must be reported as confirmed, got {:?}",
        transactions[0]
    );
    assert_eq!(fx.observed.last_balance_sats(), Some(50_000));
    assert!(height > 0);
}

#[tokio::test(flavor = "current_thread")]
async fn custom_electrum_node_scans_the_wallet_after_being_saved() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let mut fx = fixture();
    fx.node.chain().mine(5);
    let txid = fx.node.chain().pay(&fx.first_receive, Amount::from_sat(75_000));
    fx.node.chain().confirm(txid);

    // the same path the custom node screen takes: check the node, then save it
    crate::node_connect::NodeSelector::new()
        .check_and_save_node(fx.node.electrum_node("custom electrum"))
        .await
        .expect("the fake electrum node is accepted");

    send!(fx.addr.wallet_scan_and_notify(false));
    fx.observed.until("the scan to complete", is_scan_complete).await;
    fx.observed.until("the scan status to go idle", is_idle).await;
    fx.observed.settle();

    let transactions = scan_complete_transactions(&fx.observed);
    assert_eq!(transactions.len(), 1);
    assert!(
        matches!(&transactions[0], Transaction::Confirmed(confirmed) if confirmed.id().0 == txid)
    );
    assert_eq!(fx.observed.last_balance_sats(), Some(75_000));
    assert_eq!(fx.observed.ledger_states().last(), Some(&WalletLedgerState::Complete));
    assert!(
        fx.node.requests().iter().all(|request| request.protocol == Protocol::Electrum)
            && !fx.node.requests().is_empty(),
        "the scan must have gone through the saved electrum node"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn sending_is_refused_until_the_initial_scan_completes() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let mut fx = fixture();
    fx.node.chain().mine(5);
    let txid = fx.node.chain().pay(&fx.first_receive, Amount::from_sat(100_000));
    fx.node.chain().confirm(txid);
    fx.node.select_esplora();

    let (gate, _rule) = fx.node.on("/scripthash/").hold();
    send!(fx.addr.wallet_scan_and_notify(false));
    wait_until("the scan to reach the node", || gate.waiting() > 0).await;

    let during_scan = call!(fx.addr.build_tx(
        Amount::from_sat(10_000),
        Address::preview_new(),
        one_sat_vbyte_fee_rate()
    ))
    .await
    .expect("actor responds");
    assert_eq!(
        during_scan.map(|_| ()),
        Err(crate::manager::wallet_manager::Error::InitialScanIncomplete),
        "building a send before the first scan finished must be refused"
    );

    gate.open();
    fx.observed.until("the scan to complete", is_scan_complete).await;
    fx.observed.until("the scan status to go idle", is_idle).await;

    let after_scan = call!(fx.addr.build_tx(
        Amount::from_sat(10_000),
        Address::preview_new(),
        one_sat_vbyte_fee_rate()
    ))
    .await
    .expect("actor responds");
    assert!(after_scan.is_ok(), "the scanned funds must be spendable, got {after_scan:?}");
}

#[tokio::test(flavor = "current_thread")]
async fn pending_transaction_refreshes_to_confirmed_when_the_node_confirms_it() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let mut fx = fixture();
    fx.node.chain().mine(5);
    let txid = fx.node.chain().pay(&fx.first_receive, Amount::from_sat(50_000));
    fx.node.select_esplora();

    // hold the watcher's first status poll so the confirmation lands while it is in flight
    let (gate, _rule) = fx.node.on(&format!("/tx/{txid}/status")).hold();

    send!(fx.addr.wallet_scan_and_notify(false));
    fx.observed.until("the scan to complete", is_scan_complete).await;
    fx.observed.until("the scan status to go idle", is_idle).await;
    fx.observed.settle();

    let scanned = scan_complete_transactions(&fx.observed);
    assert!(
        matches!(scanned.as_slice(), [Transaction::Unconfirmed(pending)] if pending.id().0 == txid),
        "the mempool payment must be scanned as pending, got {scanned:?}"
    );
    assert_eq!(fx.observed.last_balance_sats(), Some(0), "pending funds are not confirmed yet");

    // the details screen asks to monitor the transaction it opened
    call!(fx.addr.monitor_transaction_confirmation(txid)).await.expect("monitoring starts");
    wait_until("the watcher to poll the node", || gate.waiting() > 0).await;

    fx.observed.settle();
    assert!(
        !fx.observed.seen.iter().any(|message| matches!(
            message,
            Msg::TransactionUpdated(Transaction::Confirmed(_)) | Msg::TransactionDetailsUpdated(_)
        )),
        "nothing may report a confirmation before the node has confirmed the transaction"
    );

    let height = fx.node.chain().confirm(txid);
    gate.open();

    let confirmed_details = |message: &Msg| {
        matches!(
            message,
            Msg::TransactionDetailsUpdated(details)
                if details.tx_id().0 == txid && details.confirmations().is_some()
        )
    };
    fx.observed.until("the confirmed details to be reconciled", confirmed_details).await;
    fx.observed
        .until("the confirmed transaction to be reconciled", |message| {
            matches!(
                message,
                Msg::TransactionUpdated(Transaction::Confirmed(confirmed))
                    if confirmed.id().0 == txid
            )
        })
        .await;
    fx.observed
        .until("the confirmed balance to be reconciled", |message| {
            matches!(message, Msg::WalletBalanceChanged(balance) if balance.confirmed.to_sat() == 50_000)
        })
        .await;

    let confirmations = fx
        .observed
        .seen
        .iter()
        .find_map(|message| match message {
            Msg::TransactionDetailsUpdated(details) if confirmed_details(message) => {
                details.confirmations()
            }
            _ => None,
        })
        .expect("confirmed details were reconciled");
    assert_eq!(
        confirmations,
        fx.node.chain().tip_height() - height + 1,
        "the details must show the confirmations the node reports"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn scan_against_an_offline_node_reports_the_failure_and_recovers_on_a_new_node() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let mut fx = fixture();
    fx.node.chain().mine(5);
    let txid = fx.node.chain().pay(&fx.first_receive, Amount::from_sat(20_000));
    fx.node.chain().confirm(txid);

    // an unreachable node is the selected one, like a user who picked a dead server
    fx.node.go_offline();
    fx.node.select_esplora();

    send!(fx.addr.wallet_scan_and_notify(false));
    fx.observed
        .until("the node failure to be reported", |message| {
            matches!(message, Msg::NodeConnectionFailed(_))
        })
        .await;
    fx.observed.settle();

    assert_eq!(fx.observed.count(is_scan_complete), 0);
    assert!(
        !fx.observed
            .scan_statuses()
            .iter()
            .any(|status| matches!(status, WalletScanStatus::Scanning(_))),
        "a scan that never reached a node must not report progress, got {:?}",
        fx.observed.scan_statuses()
    );
    assert!(
        !fx.observed.seen.iter().any(|message| matches!(
            message,
            Msg::WalletMetadataChanged(metadata) if metadata.internal.performed_full_scan_at.is_some()
        )),
        "a failed scan must not be recorded as the first full scan"
    );
    assert!(
        !fx.observed.ledger_states().contains(&WalletLedgerState::Complete),
        "the ledger must stay incomplete after a failed first scan"
    );

    // switching to a working node must let the same wallet finish its first scan
    let working = FakeNode::start(Network::Bitcoin);
    working.chain().mine(5);
    let working_txid = working.chain().pay(&fx.first_receive, Amount::from_sat(20_000));
    working.chain().confirm(working_txid);
    working.select_esplora();

    send!(fx.addr.wallet_scan_and_notify(false));
    fx.observed.until("the scan to complete", is_scan_complete).await;
    fx.observed.until("the scan status to go idle", is_idle).await;
    fx.observed.settle();

    assert_eq!(fx.observed.last_balance_sats(), Some(20_000));
    assert_eq!(fx.observed.ledger_states().last(), Some(&WalletLedgerState::Complete));
}
