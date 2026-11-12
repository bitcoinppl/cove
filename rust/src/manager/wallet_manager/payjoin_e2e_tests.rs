//! Pays a real BIP 77 receiver through Cove's Payjoin sender: an in-process directory and OHTTP
//! relays carry the session, a BDK wallet signs for the receiver, and the fake node holds both
//! sides' coins and checks every script of the transaction that is broadcast

use std::{sync::Arc, time::Duration};

use act_zero::call;
use bitcoin::{Amount, FeeRate as BdkFeeRate, Psbt, ScriptBuf, Transaction, Txid};
use cove_fake_node::Protocol;
use cove_types::address::AddressWithNetwork;
use payjoin::receive::v2::{Receiver, UncheckedOriginalPayload};

use super::{
    PayjoinStatus, RustWalletManager, WalletManagerAction, WalletManagerReconcileMessage as Msg,
    fake_node_tests::{Fixture, ScreenUpdates, fresh_mnemonic, receive_script},
    payjoin::{
        PayjoinConfig, PayjoinIntent, PayjoinSessionId, PayjoinTransactionKind, unix_now_secs,
    },
};
use crate::{
    manager::import_wallet_manager::import_mnemonic_with_target,
    mnemonic::MnemonicExt as _,
    test_support::{
        fake_node::wait_until,
        payjoin::{PayjoinReceiver, PayjoinServices, SilentRelay},
    },
    wallet::{Address, WalletAddressType, metadata::WalletMode},
};

/// How long these sessions wait for the receiver, long enough that no test reaches it
const SESSION_TIMEOUT: Duration = Duration::from_secs(60);

/// A hot wallet pays a receiver's pj URI, the receiver adds its own coin, and the node sees one
/// broadcast: the receiver's signed proposal, after which the sender's session record is gone.
/// The screens are told the session is negotiating until its deadline, then broadcasting and
/// completed with the proposal
#[tokio::test(flavor = "current_thread")]
async fn hot_wallet_pays_a_bip77_receiver_with_a_payjoin() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let mut fx = Fixture::signet();
    let services = PayjoinServices::start(2).await;
    let mut payment = PayjoinPayment::start(&mut fx, &services).await;

    let original = payment.receiver_gets_the_original().await;
    payment.receiver.respond(original).await;

    let completed = payment.until_completed().await;
    let broadcasts = only_broadcast(&fx);
    let payjoin = &broadcasts[0];
    let proposal_txid = payjoin.compute_txid();

    assert_eq!(
        payment.screen.payjoin_statuses(),
        vec![
            payment.negotiating(),
            payment.broadcasting(proposal_txid, PayjoinTransactionKind::Proposal),
            payment.completed(proposal_txid, PayjoinTransactionKind::Proposal),
        ],
        "the screens follow the session from negotiation to the proposal's broadcast"
    );
    assert_eq!(completed, payment.completed(proposal_txid, PayjoinTransactionKind::Proposal));
    assert_eq!(payment.status().await, completed, "a fresh screen reads the completed session");
    payment.assert_session_cleared().await;

    let receiver_coin = payment.receiver.coin_outpoint();
    assert!(
        payjoin.input.iter().any(|input| input.previous_output == receiver_coin),
        "the broadcast is the receiver's proposal, spending the receiver's coin"
    );
    assert!(payjoin.input.len() >= 2, "the proposal spends the sender's and receiver's coins");
    assert!(
        payjoin.output.iter().any(|output| output.script_pubkey == payment.change_script),
        "the proposal keeps the sender's change"
    );
    assert!(
        payjoin
            .input
            .iter()
            .filter(|input| input.previous_output != receiver_coin)
            .all(|input| !input.witness.is_empty()),
        "every sender input is signed"
    );
}

/// The user cancels while the receiver holds the Original: the wallet broadcasts the Original
/// once as an ordinary payment, and answers other requests while that broadcast is in flight
#[tokio::test(flavor = "current_thread")]
async fn cancelling_a_negotiation_broadcasts_the_original_once() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let mut fx = Fixture::signet();
    let services = PayjoinServices::start(2).await;
    let mut payment = PayjoinPayment::start(&mut fx, &services).await;

    // the receiver never answers, so only the cancel can end the wait before the deadline
    let _original = payment.receiver_gets_the_original().await;

    let (gate, rule) = fx.node.on("/tx").hold();
    payment.cancel();

    let original_txid = payment.original_txid();
    let fallback = payment.broadcasting(original_txid, PayjoinTransactionKind::Fallback);
    payment.screen.until("the fallback broadcast to start", |m| is_status(m, &fallback)).await;
    wait_until("the fallback to reach the node", || gate.waiting() == 1).await;

    // the node still holds the broadcast the cancel started, and the wallet answers anyway
    assert_eq!(payment.status().await, fallback, "the wallet answers during the broadcast");
    assert_eq!(gate.waiting(), 1, "the broadcast was still held when the wallet answered");

    drop(rule);
    gate.open();
    let completed = payment.until_completed().await;

    assert_eq!(
        payment.screen.payjoin_statuses(),
        vec![
            payment.negotiating(),
            fallback,
            payment.completed(original_txid, PayjoinTransactionKind::Fallback),
        ],
        "the cancel ends negotiation with the Original's broadcast"
    );
    assert_eq!(completed, payment.completed(original_txid, PayjoinTransactionKind::Fallback));

    let broadcasts = only_broadcast(&fx);
    assert_eq!(broadcasts[0].compute_txid(), original_txid, "the node got the Original");
    payment.assert_session_cleared().await;
}

/// A relaunch while the receiver has the Original resumes the same session: the screens are
/// told it is negotiating again, with the same id and deadline, and it still completes with the
/// receiver's proposal
#[tokio::test(flavor = "current_thread")]
async fn a_resumed_session_reports_its_status_and_completes() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let mut fx = Fixture::signet();
    let services = PayjoinServices::start(2).await;
    let mut payment = PayjoinPayment::start(&mut fx, &services).await;

    let original = payment.receiver_gets_the_original().await;

    // the app exits with the session polling, then relaunches and runs startup's resume
    call!(payment.manager.actor.abandon_payjoin_negotiation_for_test())
        .await
        .expect("the wallet actor answers");
    payment.screen.clear();
    call!(payment.manager.actor.resume_payjoin_session()).await.expect("the wallet actor answers");

    let negotiating = payment.negotiating();
    payment.screen.until("the resumed session's status", |m| is_status(m, &negotiating)).await;
    assert_eq!(payment.status().await, negotiating, "a fresh screen reads the resumed session");

    payment.receiver.respond(original).await;
    payment.until_completed().await;

    let proposal_txid = only_broadcast(&fx)[0].compute_txid();
    assert_ne!(proposal_txid, payment.original_txid(), "the resumed session sent the proposal");
    assert_eq!(
        payment.screen.payjoin_statuses(),
        vec![
            negotiating,
            payment.broadcasting(proposal_txid, PayjoinTransactionKind::Proposal),
            payment.completed(proposal_txid, PayjoinTransactionKind::Proposal),
        ],
        "the resumed session reports the same states a fresh one does"
    );
    payment.assert_session_cleared().await;
}

/// The relay never answers, so the Original's POST is still in flight when the session's
/// deadline passes: the wallet stops waiting at the deadline the screens were told, not when that
/// request gives up, and broadcasts the Original once
#[tokio::test(flavor = "current_thread")]
async fn a_session_falls_back_at_its_deadline_while_a_request_hangs() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let mut fx = Fixture::signet();
    let services = PayjoinServices::start(1).await;
    let relay = SilentRelay::start().await;

    // far shorter than the 30 seconds the sender lets its POST run
    let config = relay.sender_config(Duration::from_secs(3));
    let mut payment = PayjoinPayment::start_with(&mut fx, &services, config).await;
    wait_until("the Original's POST to reach the relay", || relay.connections() > 0).await;

    let original_txid = payment.original_txid();
    let fallback = payment.broadcasting(original_txid, PayjoinTransactionKind::Fallback);
    payment.screen.until("the fallback broadcast to start", |m| is_status(m, &fallback)).await;
    let fell_back_at = unix_now_secs();

    let deadline = payment.expires_at_secs;
    assert!(fell_back_at >= deadline, "the wallet waited for the deadline it reported");
    assert!(
        fell_back_at <= deadline + 2,
        "the wallet stopped at its deadline {deadline}, not when the POST gave up: {fell_back_at}"
    );

    payment.until_completed().await;
    let broadcasts = only_broadcast(&fx);
    assert_eq!(broadcasts[0].compute_txid(), original_txid, "the node got the Original");
    payment.assert_session_cleared().await;
}

/// Once the proposal is broadcasting, cancelling cannot send anything else: the wallet reports
/// the status it already has, both mid-broadcast and after the session completed
#[tokio::test(flavor = "current_thread")]
async fn cancelling_after_the_broadcast_starts_changes_nothing() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let mut fx = Fixture::signet();
    let services = PayjoinServices::start(2).await;
    let mut payment = PayjoinPayment::start(&mut fx, &services).await;

    let original = payment.receiver_gets_the_original().await;
    let (gate, rule) = fx.node.on("/tx").hold();
    payment.receiver.respond(original).await;
    wait_until("the proposal to reach the node", || gate.waiting() == 1).await;

    let proposal_txid = fx.node.chain().broadcasts().first().map(Transaction::compute_txid);
    assert_eq!(proposal_txid, None, "the held proposal has not been accepted yet");

    let broadcasting = payment.status().await;
    assert!(
        matches!(
            broadcasting,
            PayjoinStatus::Broadcasting { kind: PayjoinTransactionKind::Proposal, .. }
        ),
        "the proposal is broadcasting: {broadcasting:?}"
    );

    payment.screen.clear();
    payment.cancel();
    payment.screen.until("the cancel's report", |m| is_status(m, &broadcasting)).await;
    assert_eq!(payment.status().await, broadcasting, "the cancel left the broadcast alone");

    drop(rule);
    gate.open();
    let completed = payment.until_completed().await;

    payment.screen.clear();
    payment.cancel();
    payment.screen.until("the cancel's report", |m| is_status(m, &completed)).await;
    assert_eq!(payment.status().await, completed, "the cancel left the completed session alone");

    let broadcasts = only_broadcast(&fx);
    let proposal_txid = broadcasts[0].compute_txid();
    assert_ne!(proposal_txid, payment.original_txid(), "the node got the proposal");
    assert_eq!(
        completed,
        payment.completed(proposal_txid, PayjoinTransactionKind::Proposal),
        "the session completed with the proposal it was broadcasting"
    );
    assert_eq!(
        payment.screen.payjoin_statuses(),
        vec![completed],
        "the cancel after completion only reported the status"
    );
}

/// A funded hot wallet paying a receiver that has opened a session, with its Payjoin started
struct PayjoinPayment {
    manager: RustWalletManager,
    screen: ScreenUpdates,
    receiver: PayjoinReceiver,
    psbt: Psbt,
    change_script: ScriptBuf,
    session_id: PayjoinSessionId,
    expires_at_secs: u64,
}

impl PayjoinPayment {
    /// Funds a new wallet, opens a receiver session and starts paying it 40,000 sats of a
    /// 100,000 sat coin, waiting for the screens to be told the session is negotiating
    async fn start(fx: &mut Fixture, services: &PayjoinServices) -> Self {
        Self::start_with(fx, services, services.sender_config(SESSION_TIMEOUT)).await
    }

    /// Starts the payment as `start` does, sending through `config`'s relays and timeout
    async fn start_with(
        fx: &mut Fixture,
        services: &PayjoinServices,
        config: PayjoinConfig,
    ) -> Self {
        let wallet =
            import_mnemonic_with_target(fresh_mnemonic(), fx.network, WalletMode::Main).unwrap();
        let (manager, mut screen) = fx.open(wallet.id.clone());
        let funding =
            fx.node.chain().pay(&receive_script(&manager, 0).await, Amount::from_sat(100_000));
        fx.node.chain().confirm(funding);
        manager.start_wallet_scan().await.expect("the scan starts");
        screen.until_scan_settles().await;

        let receiver_wallet = fresh_mnemonic()
            .into_descriptors(None, fx.network, WalletAddressType::NativeSegwit)
            .into_create_params()
            .network(fx.network.into())
            .create_wallet_no_persist()
            .expect("the receiver wallet opens");
        let receiver = PayjoinReceiver::start(
            services,
            fx.node.chain(),
            receiver_wallet,
            Amount::from_sat(50_000),
        )
        .await;

        // the URI goes through the same parser the send screen uses
        let uri =
            AddressWithNetwork::try_new(&receiver.pj_uri().to_string()).expect("pj URI parses");
        let endpoint =
            uri.payjoin.as_ref().expect("the URI carries a pj endpoint").endpoint.clone();

        // pay part of the coin, so the Original has a change output that signing strips of its
        // derivation paths and the sender must still recognise as its own
        let recipient = Address::from(uri.address);
        let recipient_script = recipient.script_pubkey();
        let fee_rate = BdkFeeRate::from_sat_per_vb(2).expect("a valid fee rate");
        let psbt = call!(manager.actor.build_tx(Amount::from_sat(40_000), recipient, fee_rate))
            .await
            .expect("the wallet actor answers")
            .expect("the payment builds");
        let change_script = psbt
            .unsigned_tx
            .output
            .iter()
            .map(|output| output.script_pubkey.clone())
            .find(|script| *script != recipient_script)
            .expect("the payment has a change output");
        assert_eq!(psbt.unsigned_tx.output.len(), 2, "the payment has a recipient and change");

        let timeout = config.session_timeout().as_secs();
        call!(manager.actor.set_payjoin_config_for_test(config))
            .await
            .expect("the wallet actor answers");

        let intent = PayjoinIntent::new(endpoint);
        let session_id = intent.session_id.clone();
        let started_secs = unix_now_secs();
        call!(manager.actor.start_payjoin_payment_for_test(psbt.clone(), intent))
            .await
            .expect("the wallet actor answers")
            .expect("the Payjoin session starts");
        let finished_secs = unix_now_secs();

        let found = screen
            .until("the session to start negotiating", |message| {
                matches!(message, Msg::PayjoinStatusChanged(PayjoinStatus::Negotiating { .. }))
            })
            .await;
        let PayjoinStatus::Negotiating { session_id: reported_id, expires_at_secs } =
            screen.payjoin_statuses()[0].clone()
        else {
            unreachable!("message {found} is a negotiating status");
        };

        assert_eq!(reported_id, session_id, "the status names the session the screen started");
        assert!(
            (started_secs + timeout..=finished_secs + timeout).contains(&expires_at_secs),
            "the deadline is the session timeout after the session started"
        );

        Self { manager, screen, receiver, psbt, change_script, session_id, expires_at_secs }
    }

    /// The txid of the payment as built, which signing does not change for segwit inputs
    fn original_txid(&self) -> Txid {
        self.psbt.unsigned_tx.compute_txid()
    }

    async fn receiver_gets_the_original(&mut self) -> Receiver<UncheckedOriginalPayload> {
        tokio::time::timeout(Duration::from_secs(30), self.receiver.wait_for_original())
            .await
            .expect("the sender posts its Original PSBT")
    }

    fn cancel(&self) {
        let cancel = WalletManagerAction::CancelPayjoin(self.session_id.clone());
        self.manager.dispatch(cancel).expect("the cancel is dispatched");
    }

    async fn status(&self) -> PayjoinStatus {
        self.manager.payjoin_status().await.expect("the wallet actor answers")
    }

    /// Waits for the screens to be told the session completed, returning that status
    async fn until_completed(&mut self) -> PayjoinStatus {
        self.screen
            .until("the session to complete", |message| {
                matches!(message, Msg::PayjoinStatusChanged(PayjoinStatus::Completed { .. }))
            })
            .await;

        self.screen.payjoin_statuses().pop().expect("the completed status was seen")
    }

    async fn assert_session_cleared(&self) {
        let session = call!(self.manager.actor.payjoin_session_for_test())
            .await
            .expect("the wallet actor answers");
        assert!(session.is_none(), "the session record is deleted after the broadcast");
    }

    fn negotiating(&self) -> PayjoinStatus {
        PayjoinStatus::Negotiating {
            session_id: self.session_id.clone(),
            expires_at_secs: self.expires_at_secs,
        }
    }

    fn broadcasting(&self, txid: Txid, kind: PayjoinTransactionKind) -> PayjoinStatus {
        PayjoinStatus::Broadcasting {
            session_id: self.session_id.clone(),
            txid: Arc::new(txid.into()),
            kind,
        }
    }

    fn completed(&self, txid: Txid, kind: PayjoinTransactionKind) -> PayjoinStatus {
        PayjoinStatus::Completed {
            session_id: self.session_id.clone(),
            txid: Arc::new(txid.into()),
            kind,
        }
    }
}

fn is_status(message: &Msg, status: &PayjoinStatus) -> bool {
    matches!(message, Msg::PayjoinStatusChanged(seen) if seen == status)
}

/// The one transaction the node accepted, checking the wallet sent exactly one broadcast
///
/// The chain keeps each txid once, so a repeated send of the same transaction shows only in the
/// request log, where only a broadcast's POST has the bare `/tx` path
fn only_broadcast(fx: &Fixture) -> Vec<Transaction> {
    let broadcast_requests = fx
        .node
        .requests()
        .into_iter()
        .filter(|request| request.protocol == Protocol::Esplora && request.name == "/tx")
        .count();
    assert_eq!(broadcast_requests, 1, "the wallet sent exactly one broadcast");

    // the fake node records a broadcast only after every input's script verifies
    let broadcasts = fx.node.chain().broadcasts();
    assert_eq!(broadcasts.len(), 1, "the node accepted exactly one transaction");

    broadcasts
}
