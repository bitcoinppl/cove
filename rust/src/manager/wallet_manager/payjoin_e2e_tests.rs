//! Pays a real BIP 77 receiver through Cove's Payjoin sender: an in-process directory and OHTTP
//! relays carry the session, a BDK wallet signs for the receiver, and the fake node holds both
//! sides' coins and checks every script of the transaction that is broadcast

use std::{sync::Arc, time::Duration};

use act_zero::call;
use bitcoin::{Amount, FeeRate as BdkFeeRate, Psbt, ScriptBuf, Transaction, Txid};
use cove_fake_node::Protocol;
use cove_types::{address::AddressWithNetwork, confirm::ConfirmDetails};
use payjoin::receive::v2::{Receiver, UncheckedOriginalPayload};
use url::Url;

use super::{
    PayjoinStatus, RustWalletManager, WalletManagerAction, WalletManagerReconcileMessage as Msg,
    fake_node_tests::{Fixture, ScreenUpdates, fresh_mnemonic, receive_script},
    payjoin::{
        OhttpRelays, PayjoinConfig, PayjoinFeeContribution, PayjoinFeeHelp, PayjoinFeePaid,
        PayjoinIntent, PayjoinSessionId, PayjoinTransactionKind, unix_now_secs,
    },
};
use crate::{
    manager::import_wallet_manager::import_mnemonic_with_target,
    mnemonic::MnemonicExt as _,
    network::Network,
    router::UnsignedPaymentMode,
    test_support::{
        fake_node::{unreachable_url, wait_until},
        payjoin::{PayjoinReceiver, PayjoinServices, SilentRelay, TestRelay, TestRelayBehavior},
    },
    wallet::{Address, WalletAddressType, metadata::WalletMode},
};

mod crash;
mod send_flow;

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
    let proposal = payjoin.clone();
    let proposal_txid = payjoin.compute_txid();

    assert_eq!(
        payment.screen.payjoin_statuses(),
        vec![
            payment.negotiating(),
            payment.broadcasting(proposal_txid, PayjoinTransactionKind::Proposal),
            payment.completed(&proposal, PayjoinTransactionKind::Proposal),
        ],
        "the screens follow the session from negotiation to the proposal's broadcast"
    );
    assert_eq!(completed, payment.completed(&proposal, PayjoinTransactionKind::Proposal));
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

    // the receiver's coin costs more than the Original's fee rate covers, so it takes the
    // whole offer: one P2WPKH input at 2 sat/vB
    let offered = payment.fee_contribution.clone().expect("a payment with change offers a fee");
    assert_eq!(offered.max().as_sats(), 136, "one 68 vB input at 2 sat/vB");
    assert_eq!(
        payment.change_in(&proposal),
        payment.change_in(&payment.original()) - offered.max().0,
        "the receiver took exactly the offer from the change, and nothing else"
    );

    let original_fee = payment.details.fee_total.as_sats();
    let PayjoinStatus::Completed { fee_paid: Some(fee_paid), .. } = &completed else {
        panic!("the completed status carries the fee paid: {completed:?}");
    };
    assert_eq!(fee_paid.total.as_sats(), original_fee + 136, "the Original's fee plus the offer");
    assert_eq!(fee_paid.fee_help.as_sats(), 136);

    // the proposal's own fee also includes the receiver's share, which details must not show
    let (fee, sent) = payment.shown_fee_and_sent(proposal_txid).await;
    assert_eq!(fee, Some(original_fee + 136), "details show the fee this wallet paid");
    assert_eq!(sent, Some(40_000), "details show the amount the receiver asked for");
}

/// With fee help off, the payment offers the receiver nothing: the receiver pays for its own
/// coin from its own output, and the wallet pays exactly the fee the confirm screen showed
#[tokio::test(flavor = "current_thread")]
async fn without_fee_help_the_wallet_pays_only_the_original_fee() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let mut fx = Fixture::signet();
    let services = PayjoinServices::start(2).await;
    let config = services.sender_config(SESSION_TIMEOUT);
    let mut payment =
        PayjoinPayment::start_offering(&mut fx, &services, config, PayjoinFeeHelp::DontHelp).await;
    assert!(payment.fee_contribution.is_none(), "fee help off offers nothing");

    let original = payment.receiver_gets_the_original().await;
    payment.receiver.respond(original).await;
    let completed = payment.until_completed().await;

    let proposal = only_broadcast(&fx)[0].clone();
    assert_ne!(proposal.compute_txid(), payment.original_txid(), "the receiver joined");
    assert_eq!(
        payment.change_in(&proposal),
        payment.change_in(&payment.original()),
        "the receiver took nothing from the change"
    );

    let original_fee = payment.details.fee_total.as_sats();
    assert_eq!(completed, payment.completed(&proposal, PayjoinTransactionKind::Proposal));
    let PayjoinStatus::Completed { fee_paid: Some(fee_paid), .. } = &completed else {
        panic!("the completed status carries the fee paid: {completed:?}");
    };
    assert_eq!(fee_paid.total.as_sats(), original_fee, "exactly the confirm screen's fee");
    assert_eq!(fee_paid.fee_help.as_sats(), 0);

    let (fee, sent) = payment.shown_fee_and_sent(proposal.compute_txid()).await;
    assert_eq!(fee, Some(original_fee));
    assert_eq!(sent, Some(40_000));
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
            payment.completed(&payment.original(), PayjoinTransactionKind::Fallback),
        ],
        "the cancel ends negotiation with the Original's broadcast"
    );
    assert_eq!(completed, payment.completed(&payment.original(), PayjoinTransactionKind::Fallback));

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
    payment.until_sender_is_polling().await;

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

    let proposal = only_broadcast(&fx)[0].clone();
    let proposal_txid = proposal.compute_txid();
    assert_ne!(proposal_txid, payment.original_txid(), "the resumed session sent the proposal");
    assert_eq!(
        payment.screen.payjoin_statuses(),
        vec![
            negotiating,
            payment.broadcasting(proposal_txid, PayjoinTransactionKind::Proposal),
            payment.completed(&proposal, PayjoinTransactionKind::Proposal),
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
    let proposal = broadcasts[0].clone();
    assert_ne!(proposal.compute_txid(), payment.original_txid(), "the node got the proposal");
    assert_eq!(
        completed,
        payment.completed(&proposal, PayjoinTransactionKind::Proposal),
        "the session completed with the proposal it was broadcasting"
    );
    assert_eq!(
        payment.screen.payjoin_statuses(),
        vec![completed],
        "the cancel after completion only reported the status"
    );
}

/// The receiver checks the Original and rejects it with a well-known error: the sender
/// broadcasts the Original once, reports the session completed with it, and is free to send
/// again
#[tokio::test(flavor = "current_thread")]
async fn a_rejecting_receiver_gets_the_original_once() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let mut fx = Fixture::signet();
    let services = PayjoinServices::start(2).await;
    let mut payment = PayjoinPayment::start(&mut fx, &services).await;

    let original = payment.receiver_gets_the_original().await;
    payment.receiver.reject(original).await;

    let completed = payment.until_completed().await;
    let original_txid = payment.original_txid();
    assert_eq!(
        payment.screen.payjoin_statuses(),
        vec![
            payment.negotiating(),
            payment.broadcasting(original_txid, PayjoinTransactionKind::Fallback),
            payment.completed(&payment.original(), PayjoinTransactionKind::Fallback),
        ],
        "the rejection ends negotiation with the Original's broadcast"
    );
    assert_eq!(completed, payment.completed(&payment.original(), PayjoinTransactionKind::Fallback));

    let broadcasts = only_broadcast(&fx);
    assert_eq!(broadcasts[0].compute_txid(), original_txid, "the node got the Original");
    payment.assert_session_cleared().await;
    payment.assert_send_gate_open().await;
}

/// No relay carries the Original's POST: one answers 502, one redirects to a relay the wallet
/// was never given, and one is not listening. Each is tried once, the redirect is not followed,
/// and the Original goes out at once instead of waiting for the deadline
#[tokio::test(flavor = "current_thread")]
async fn every_relay_refusing_the_post_sends_the_original_at_once() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let mut fx = Fixture::signet();
    let services = PayjoinServices::start(1).await;
    let client = services.http_client();
    let down = TestRelay::start(TestRelayBehavior::Down, client.clone()).await;
    let unlisted = TestRelay::forwarding_to(&services, 0).await;
    let redirecting =
        TestRelay::start(TestRelayBehavior::Redirect(unlisted.url().clone()), client).await;
    let unreachable = Url::parse(&unreachable_url()).expect("a valid URL");

    let relays = [down.url().clone(), redirecting.url().clone(), unreachable];
    let relays = OhttpRelays::loopback_for_test(relays)
        .expect("loopback relays are allowed in tests")
        .in_listed_order();
    let config = PayjoinConfig::new(relays, SESSION_TIMEOUT);
    let mut payment = PayjoinPayment::start_with(&mut fx, &services, config).await;

    let completed = payment.until_completed().await;
    let original_txid = payment.original_txid();
    assert!(
        unix_now_secs() < payment.expires_at_secs,
        "the Original went out without waiting for the session's deadline"
    );
    assert_eq!(
        payment.screen.payjoin_statuses(),
        vec![
            payment.negotiating(),
            payment.broadcasting(original_txid, PayjoinTransactionKind::Fallback),
            payment.completed(&payment.original(), PayjoinTransactionKind::Fallback),
        ],
        "the failed POST ends negotiation with the Original's broadcast"
    );
    assert_eq!(completed, payment.completed(&payment.original(), PayjoinTransactionKind::Fallback));

    assert_eq!(down.requests(), 1, "the relay that answered 502 was tried once");
    assert_eq!(redirecting.requests(), 1, "the redirecting relay was tried once");
    assert_eq!(unlisted.requests(), 0, "the redirect to an unlisted relay was not followed");

    let broadcasts = only_broadcast(&fx);
    assert_eq!(broadcasts[0].compute_txid(), original_txid, "the node got the Original");
    payment.assert_session_cleared().await;
    payment.assert_send_gate_open().await;
}

/// Every relay goes down after the POST, so each poll fails. The session keeps polling only
/// the relay that carried its POST, never moves to another, and sends the Original once at the
/// deadline the screens were told
#[tokio::test(flavor = "current_thread")]
async fn every_relay_going_down_while_polling_sends_the_original_at_the_deadline() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let mut fx = Fixture::signet();
    let services = PayjoinServices::start(2).await;
    let adopted = TestRelay::forwarding_to(&services, 0).await;
    let other = TestRelay::forwarding_to(&services, 1).await;

    let session_timeout = Duration::from_secs(8);
    let config = TestRelay::sender_config(&[&adopted, &other], session_timeout);
    let mut payment = PayjoinPayment::start_with(&mut fx, &services, config).await;
    let _original = payment.receiver_gets_the_original().await;
    payment.until_sender_is_polling().await;

    adopted.set_behavior(TestRelayBehavior::Down);
    other.set_behavior(TestRelayBehavior::Down);
    let before_outage = adopted.requests();
    assert!(unix_now_secs() < payment.expires_at_secs, "the outage began before the deadline");

    // timed when the fallback starts, since completing it adds the broadcast and wallet save
    let original_txid = payment.original_txid();
    let fallback = payment.broadcasting(original_txid, PayjoinTransactionKind::Fallback);
    payment.screen.until("the fallback broadcast to start", |m| is_status(m, &fallback)).await;
    let fell_back_at = unix_now_secs();
    let deadline = payment.expires_at_secs;
    assert!(fell_back_at >= deadline, "the wallet waited for the deadline it reported");
    assert!(
        fell_back_at <= deadline + 2,
        "the wallet stopped at its deadline {deadline}, not later: {fell_back_at}"
    );

    let completed = payment.until_completed().await;
    assert_eq!(
        payment.screen.payjoin_statuses(),
        vec![
            payment.negotiating(),
            payment.broadcasting(original_txid, PayjoinTransactionKind::Fallback),
            payment.completed(&payment.original(), PayjoinTransactionKind::Fallback),
        ],
        "the outage ends negotiation with the Original's broadcast at the deadline"
    );
    assert_eq!(completed, payment.completed(&payment.original(), PayjoinTransactionKind::Fallback));

    assert!(
        adopted.requests() > before_outage,
        "the session kept polling the relay that carried its POST during the outage"
    );
    assert_eq!(other.requests(), 0, "the session never moved to another relay");

    let broadcasts = only_broadcast(&fx);
    assert_eq!(broadcasts[0].compute_txid(), original_txid, "the node got the Original");
    payment.assert_session_cleared().await;
    payment.assert_send_gate_open().await;
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

    /// The payment as the confirm screen showed it, before any Payjoin
    details: ConfirmDetails,

    /// What the payment offered the receiver toward its coin's fee
    fee_contribution: Option<Arc<PayjoinFeeContribution>>,
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
        Self::start_offering(fx, services, config, PayjoinFeeHelp::UpToOneInput).await
    }

    /// Starts the payment as `start_with` does, offering the receiver what `fee_help` allows
    async fn start_offering(
        fx: &mut Fixture,
        services: &PayjoinServices,
        config: PayjoinConfig,
        fee_help: PayjoinFeeHelp,
    ) -> Self {
        let wallet =
            import_mnemonic_with_target(fresh_mnemonic(), fx.network, WalletMode::Main).unwrap();
        let (manager, mut screen) = fx.open(wallet.id.clone());
        let funding =
            fx.node.chain().pay(&receive_script(&manager, 0).await, Amount::from_sat(100_000));
        fx.node.chain().confirm(funding);
        manager.start_wallet_scan().await.expect("the scan starts");
        screen.until_scan_settles().await;

        let receiver = PayjoinReceiver::start(
            services,
            fx.node.chain(),
            fresh_receiver_wallet(fx.network),
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

        // the offer is fixed with the confirm screen's details, as the send flow does
        let details = call!(manager.actor.get_confirm_details(psbt.clone(), fee_rate))
            .await
            .expect("the wallet actor answers")
            .expect("the confirm details build");
        let fee_contribution =
            call!(manager.actor.payjoin_fee_contribution(Arc::new(details.clone()), fee_help))
                .await
                .expect("the wallet actor answers");

        let intent = PayjoinIntent::new(endpoint, fee_contribution);
        let fee_contribution = intent.fee_contribution.clone();
        let session_id = intent.session_id.clone();
        let started_secs = unix_now_secs();
        let mode = UnsignedPaymentMode::Payjoin { intent };
        call!(manager.actor.initiate_payment(psbt.clone(), mode))
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

        Self {
            manager,
            screen,
            receiver,
            psbt,
            change_script,
            session_id,
            expires_at_secs,
            details,
            fee_contribution,
        }
    }

    /// The Original as built, which has the same txid and outputs as the signed one
    fn original(&self) -> Transaction {
        self.psbt.unsigned_tx.clone()
    }

    /// The change `tx` pays back to the wallet
    fn change_in(&self, tx: &Transaction) -> Amount {
        tx.output
            .iter()
            .find(|output| output.script_pubkey == self.change_script)
            .expect("the transaction pays the wallet's change")
            .value
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

    /// Waits until the sender has saved the directory's answer to its POST
    ///
    /// The directory hands the receiver the Original as soon as it stores the POST, a moment
    /// before the sender saves that answer, and a relaunch inside that gap falls back by design
    /// because re-posting is not retry-safe
    async fn until_sender_is_polling(&self) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let polling = call!(self.manager.actor.payjoin_session_is_polling_for_test())
                .await
                .expect("the wallet actor answers");
            if polling {
                return;
            }

            assert!(tokio::time::Instant::now() < deadline, "the sender saves the POST's answer");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
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

    /// Checks a new payment passes the send gate, which a stored session record would block
    async fn assert_send_gate_open(&self) {
        let gate = call!(self.manager.actor.check_payjoin_send_gate_for_test())
            .await
            .expect("the wallet actor answers");
        assert!(gate.is_ok(), "the send gate lets a new payment start: {gate:?}");
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

    /// The completed status for broadcasting `tx`, whose fee is the Original's plus whatever
    /// the receiver took from the change, worked out from the outputs alone
    fn completed(&self, tx: &Transaction, kind: PayjoinTransactionKind) -> PayjoinStatus {
        let fee_help = self.change_in(&self.original()) - self.change_in(tx);
        let total = self.details.fee_total.0 + fee_help;

        PayjoinStatus::Completed {
            session_id: self.session_id.clone(),
            txid: Arc::new(tx.compute_txid().into()),
            kind,
            fee_paid: Some(PayjoinFeePaid {
                total: Arc::new(total.into()),
                fee_help: Arc::new(fee_help.into()),
            }),
        }
    }

    /// The fee the wallet's transaction details show for `txid`, and the amount they show sent
    async fn shown_fee_and_sent(&self, txid: Txid) -> (Option<u64>, Option<u64>) {
        let presentation = call!(self.manager.actor.transaction_details(txid.into()))
            .await
            .expect("the wallet actor answers");
        let details = presentation.details();

        (details.fee.map(|fee| fee.as_sats()), details.sent_sans_fee().map(|sent| sent.as_sats()))
    }
}

/// A wallet for the receiver's side, which nobody has used
fn fresh_receiver_wallet(network: Network) -> bdk_wallet::Wallet {
    fresh_mnemonic()
        .into_descriptors(None, network, WalletAddressType::NativeSegwit)
        .into_create_params()
        .network(network.into())
        .create_wallet_no_persist()
        .expect("the receiver wallet opens")
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
