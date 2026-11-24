//! Which OHTTP relays a Payjoin sender session sends through: it adopts the first relay whose
//! answer passes `process_response`, sends every later request through that relay alone, even
//! while it fails, and keeps it when the session resumes after a restart (#910)

use std::time::Duration;

use act_zero::call;
use bitcoin::{Amount, FeeRate as BdkFeeRate, Psbt};
use cove_types::{address::AddressWithNetwork, fees::FeeRate};
use payjoin::receive::v2::{Receiver, UncheckedOriginalPayload};
use url::Url;

use super::{
    RustWalletManager,
    fake_node_tests::{Fixture, fresh_mnemonic, receive_script},
    payjoin::{PayjoinConfig, PayjoinIntent, unix_now_secs},
};
use crate::{
    manager::import_wallet_manager::import_mnemonic_with_target,
    mnemonic::MnemonicExt as _,
    router::UnsignedPaymentMode,
    test_support::{
        fake_node::wait_until,
        payjoin::{PayjoinReceiver, PayjoinServices, TestRelay, TestRelayBehavior},
    },
    wallet::{Address, WalletAddressType, metadata::WalletMode},
};

const SESSION_TIMEOUT: Duration = Duration::from_secs(60);

/// The first relay is down, so the POST moves on to the second, and the session then sends
/// every poll through the second without trying the first again
#[tokio::test(flavor = "current_thread")]
async fn a_session_skips_a_down_relay_and_keeps_the_next_one() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let mut fx = Fixture::signet();
    let services = PayjoinServices::start(1).await;
    let down = TestRelay::start(TestRelayBehavior::Down, services.http_client()).await;
    let working = TestRelay::forwarding_to(&services, 0).await;
    let mut payment = Payment::prepare(&mut fx, &services).await;

    payment.start(TestRelay::sender_config(&[&down, &working], SESSION_TIMEOUT)).await;
    let original = payment.wait_for_original().await;

    // the POST and at least two polls
    wait_until("two polls through the working relay", || working.requests() >= 3).await;
    assert_eq!(payment.saved_relay().await.as_ref(), Some(working.url()));

    payment.finish(&fx, original).await;

    assert_eq!(down.requests(), 1, "only the POST's first attempt reached the down relay");
    assert!(working.requests() >= 3, "the POST and every poll went through the working relay");
}

/// The first relay answers the POST, but its answer fails `process_response`, so the session
/// does not adopt it and moves on to the second relay for the POST and every poll
#[tokio::test(flavor = "current_thread")]
async fn a_relay_whose_answer_fails_processing_is_not_adopted() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let mut fx = Fixture::signet();
    let services = PayjoinServices::start(1).await;
    let garbled = TestRelay::start(TestRelayBehavior::Garbled, services.http_client()).await;
    let working = TestRelay::forwarding_to(&services, 0).await;
    let mut payment = Payment::prepare(&mut fx, &services).await;

    payment.start(TestRelay::sender_config(&[&garbled, &working], SESSION_TIMEOUT)).await;
    let original = payment.wait_for_original().await;

    wait_until("two polls through the working relay", || working.requests() >= 3).await;
    assert_eq!(payment.saved_relay().await.as_ref(), Some(working.url()));

    payment.finish(&fx, original).await;

    assert_eq!(garbled.requests(), 1, "only the POST's first attempt reached the garbled relay");
    assert!(working.requests() >= 3, "the POST and every poll went through the working relay");
}

/// A session that adopted the second of its relays keeps polling through it after a restart,
/// even though the relay listed first is up and would be tried first by a new session
#[tokio::test(flavor = "current_thread")]
async fn a_resumed_session_keeps_its_saved_relay() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let mut fx = Fixture::signet();
    let services = PayjoinServices::start(2).await;
    let down = TestRelay::start(TestRelayBehavior::Down, services.http_client()).await;
    let first = TestRelay::forwarding_to(&services, 0).await;
    let adopted = TestRelay::forwarding_to(&services, 1).await;
    let mut payment = Payment::prepare(&mut fx, &services).await;

    payment.start(TestRelay::sender_config(&[&down, &adopted], SESSION_TIMEOUT)).await;
    let original = payment.wait_for_original().await;
    wait_until("a poll through the adopted relay", || adopted.requests() >= 2).await;
    assert_eq!(payment.saved_relay().await.as_ref(), Some(adopted.url()));

    // the relaunched app lists a working relay first, which a session without a saved relay
    // would adopt
    let before_restart = adopted.requests();
    call!(payment.manager.actor.restart_payjoin_session_for_test(TestRelay::sender_config(
        &[&first, &adopted],
        SESSION_TIMEOUT
    )))
    .await
    .expect("the wallet actor answers");

    wait_until("two polls through the adopted relay after the restart", || {
        adopted.requests() >= before_restart + 2
    })
    .await;

    payment.finish(&fx, original).await;

    assert_eq!(first.requests(), 0, "the resumed session never tried the first listed relay");
    assert_eq!(down.requests(), 1, "only the POST's first attempt reached the down relay");
}

/// A session whose saved relay is no longer configured after a restart broadcasts the original
/// transaction rather than send the session through a second relay
#[tokio::test(flavor = "current_thread")]
async fn a_resumed_session_whose_relay_was_removed_falls_back() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let mut fx = Fixture::signet();
    let services = PayjoinServices::start(2).await;
    let adopted = TestRelay::forwarding_to(&services, 0).await;
    let other = TestRelay::forwarding_to(&services, 1).await;
    let mut payment = Payment::prepare(&mut fx, &services).await;

    payment.start(TestRelay::sender_config(&[&adopted], SESSION_TIMEOUT)).await;
    payment.wait_for_original().await;
    wait_until("a poll through the adopted relay", || adopted.requests() >= 2).await;

    call!(
        payment
            .manager
            .actor
            .restart_payjoin_session_for_test(TestRelay::sender_config(&[&other], SESSION_TIMEOUT))
    )
    .await
    .expect("the wallet actor answers");

    payment.wait_for_fallback(&fx).await;
    assert_eq!(other.requests(), 0, "the resumed session never tried the other relay");
}

/// After the POST, the adopted relay goes down, so the session keeps retrying it until the
/// session times out and broadcasts the original transaction, never trying the other relay
#[tokio::test(flavor = "current_thread")]
async fn a_failing_adopted_relay_is_retried_until_the_session_falls_back() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let mut fx = Fixture::signet();
    let services = PayjoinServices::start(2).await;
    let adopted = TestRelay::forwarding_to(&services, 0).await;
    let other = TestRelay::forwarding_to(&services, 1).await;
    let mut payment = Payment::prepare(&mut fx, &services).await;

    payment.start(TestRelay::sender_config(&[&adopted, &other], Duration::from_secs(10))).await;
    payment.wait_for_original().await;
    wait_until("a poll through the adopted relay", || adopted.requests() >= 2).await;
    assert_eq!(payment.saved_relay().await.as_ref(), Some(adopted.url()));

    adopted.set_behavior(TestRelayBehavior::Down);
    let before_outage = adopted.requests();
    payment.wait_for_fallback(&fx).await;

    assert!(
        adopted.requests() >= before_outage + 2,
        "the session retried the adopted relay while it was down"
    );
    assert_eq!(other.requests(), 0, "the session never switched to the other relay");
}

/// After the POST, the adopted relay redirects every request to the other relay. The session
/// does not follow the redirect, so the other relay never sees the sender, and the session
/// falls back once it times out
#[tokio::test(flavor = "current_thread")]
async fn a_redirect_from_the_adopted_relay_is_not_followed() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let mut fx = Fixture::signet();
    let services = PayjoinServices::start(2).await;
    let adopted = TestRelay::forwarding_to(&services, 0).await;
    let other = TestRelay::forwarding_to(&services, 1).await;
    let mut payment = Payment::prepare(&mut fx, &services).await;

    payment.start(TestRelay::sender_config(&[&adopted, &other], Duration::from_secs(10))).await;
    payment.wait_for_original().await;
    wait_until("a poll through the adopted relay", || adopted.requests() >= 2).await;

    adopted.set_behavior(TestRelayBehavior::Redirect(other.url().clone()));
    let before_redirects = adopted.requests();
    payment.wait_for_fallback(&fx).await;

    assert!(
        adopted.requests() >= before_redirects + 2,
        "the session kept polling the adopted relay while it redirected"
    );
    assert_eq!(other.requests(), 0, "no request followed the redirect to the other relay");
}

/// The adopted relay stops answering mid-session, so a poll hangs on it. The session's
/// deadline still falls back on time instead of waiting for that poll to give up, and the
/// session never tries the other relay
#[tokio::test(flavor = "current_thread")]
async fn a_hanging_adopted_relay_still_falls_back_at_the_deadline() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let mut fx = Fixture::signet();
    let services = PayjoinServices::start(2).await;
    let adopted = TestRelay::forwarding_to(&services, 0).await;
    let other = TestRelay::forwarding_to(&services, 1).await;
    let mut payment = Payment::prepare(&mut fx, &services).await;

    // far shorter than the 35 seconds the sender lets a poll run
    let session_timeout = Duration::from_secs(10);
    payment.start(TestRelay::sender_config(&[&adopted, &other], session_timeout)).await;
    payment.wait_for_original().await;
    wait_until("a poll through the adopted relay", || adopted.requests() >= 2).await;
    assert_eq!(payment.saved_relay().await.as_ref(), Some(adopted.url()));

    let deadline = payment.created_at_secs().await + session_timeout.as_secs();
    adopted.set_behavior(TestRelayBehavior::Silent);
    wait_until("a poll to hang on the adopted relay", || adopted.held() == 1).await;
    assert!(
        unix_now_secs() < deadline,
        "a poll was hanging on the adopted relay before the deadline {deadline}"
    );

    payment.wait_for_fallback(&fx).await;
    let fell_back_at = unix_now_secs();

    assert!(fell_back_at >= deadline, "the session waited for its deadline {deadline}");
    assert!(
        fell_back_at <= deadline + 2,
        "the session fell back at its deadline {deadline}, not when the poll gave up: {fell_back_at}"
    );
    assert_eq!(adopted.held(), 1, "no poll started while one hung on the adopted relay");
    assert_eq!(other.requests(), 0, "the session never switched to the other relay");
}

/// A funded hot wallet with a Max payment to a BIP 77 receiver, ready to start as a Payjoin
struct Payment {
    manager: RustWalletManager,
    receiver: PayjoinReceiver,
    psbt: Psbt,
    intent: PayjoinIntent,
}

impl Payment {
    async fn prepare(fx: &mut Fixture, services: &PayjoinServices) -> Self {
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

        let uri =
            AddressWithNetwork::try_new(&receiver.pj_uri().to_string()).expect("pj URI parses");
        let endpoint =
            uri.payjoin.as_ref().expect("the URI carries a pj endpoint").endpoint.clone();
        let intent = PayjoinIntent::new(endpoint);

        // a Max payment, so the relay tests do not depend on the change detection the
        // end-to-end tests cover
        let fee_rate = FeeRate::from(BdkFeeRate::from_sat_per_vb(2).expect("a valid fee rate"));
        let psbt =
            call!(manager.actor.build_ephemeral_drain_tx(Address::from(uri.address), fee_rate))
                .await
                .expect("the wallet actor answers")
                .expect("the Max payment builds");

        Self { manager, receiver, psbt, intent }
    }

    async fn start(&self, config: PayjoinConfig) {
        call!(self.manager.actor.set_payjoin_config_for_test(config))
            .await
            .expect("the wallet actor answers");
        let mode = UnsignedPaymentMode::Payjoin { intent: self.intent.clone() };
        call!(self.manager.actor.initiate_payment(self.psbt.clone(), mode))
            .await
            .expect("the wallet actor answers")
            .expect("the Payjoin session starts");
    }

    async fn wait_for_original(&mut self) -> Receiver<UncheckedOriginalPayload> {
        tokio::time::timeout(Duration::from_secs(30), self.receiver.wait_for_original())
            .await
            .expect("the sender posts its Original PSBT")
    }

    /// The relay saved on the session record
    async fn saved_relay(&self) -> Option<Url> {
        call!(self.manager.actor.payjoin_session_for_test())
            .await
            .expect("the wallet actor answers")
            .expect("the session record exists")
            .ohttp_relay
    }

    /// When the session record says the session started, which its deadline counts from
    async fn created_at_secs(&self) -> u64 {
        call!(self.manager.actor.payjoin_session_for_test())
            .await
            .expect("the wallet actor answers")
            .expect("the session record exists")
            .created_at_secs
            .expect("a new session records its start")
    }

    /// Waits for the sender to give up on the Payjoin and broadcast the original transaction
    async fn wait_for_fallback(&self, fx: &Fixture) {
        wait_until("the original transaction to be broadcast", || {
            !fx.node.chain().broadcasts().is_empty()
        })
        .await;

        let broadcasts = fx.node.chain().broadcasts();
        assert_eq!(broadcasts.len(), 1, "the node accepted exactly one transaction");
        assert_eq!(
            broadcasts[0].compute_txid(),
            self.psbt.unsigned_tx.compute_txid(),
            "the broadcast is the original transaction"
        );
    }

    /// Answers with the receiver's proposal and waits for the sender to broadcast it
    async fn finish(&self, fx: &Fixture, original: Receiver<UncheckedOriginalPayload>) {
        self.receiver.respond(original).await;

        wait_until("the Payjoin to be broadcast", || !fx.node.chain().broadcasts().is_empty())
            .await;

        let broadcasts = fx.node.chain().broadcasts();
        let receiver_coin = self.receiver.coin_outpoint();
        assert_eq!(broadcasts.len(), 1, "the node accepted exactly one transaction");
        assert!(
            broadcasts[0].input.iter().any(|input| input.previous_output == receiver_coin),
            "the broadcast is the receiver's proposal, spending the receiver's coin"
        );
    }
}
