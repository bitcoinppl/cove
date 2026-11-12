//! Stops the sender's app process right after each durable step of a Payjoin send, relaunches
//! the wallet from what that step left behind, and checks the session still ends with exactly one
//! of the proposal or the Original on the node, its record deleted and new sends allowed (#1004)
//!
//! Each run counts the store writes and node broadcasts of one send, stops the process after the
//! chosen one, kills its wallet actor without the graceful shutdown a quitting app would get, and
//! starts a new actor on the same wallet. The relaunched wallet may also meet one more failure
//! while it recovers, and is then helped along only as a user would help it: by trying to send
//! again, which runs the send gate

use std::{
    collections::BTreeSet,
    fmt,
    str::FromStr as _,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use act_zero::{Addr, AddrLike as _, call};
use bitcoin::{Amount, FeeRate as BdkFeeRate, OutPoint, Psbt, Transaction, Txid};
use cove_device::keychain::WalletSecret;
use cove_fake_node::{Protocol, Scripted};
use cove_tokio::task::spawn_actor;
use cove_types::address::AddressWithNetwork;
use parking_lot::RwLock;

use super::{
    super::{
        ReconcileChannel, WalletActor, WalletScanStatus, WalletSnapshot,
        payjoin::{PayjoinConfig, PayjoinIntent, PayjoinSessionStore, PayjoinTransactionKind},
    },
    Fixture, Msg, PayjoinStatus, SESSION_TIMEOUT, ScreenUpdates, fresh_mnemonic,
    fresh_receiver_wallet,
};
use crate::{
    database::wallet_data::WalletDataDb,
    keychain::Keychain,
    manager::import_wallet_manager::import_mnemonic_with_target,
    router::UnsignedPaymentMode,
    test_support::{
        loaded_host_wait,
        payjoin::{PayjoinReceiver, PayjoinServices, TestRelay, TestRelayBehavior},
        payjoin_crash::{CrashSwitch, DurableStep, FlakyStore},
    },
    wallet::{Address, Wallet, metadata::WalletId, metadata::WalletMode},
};

/// The durable steps of a send that nothing stops: four session writes up to the receiver's
/// answer, the terminal intent, the broadcast, then deleting the record
const STEPS_OF_A_SEND: [DurableStep; 8] = [
    DurableStep::StoreWrite,
    DurableStep::StoreWrite,
    DurableStep::StoreWrite,
    DurableStep::StoreWrite,
    DurableStep::StoreWrite,
    DurableStep::BroadcastAccepted,
    DurableStep::BroadcastAnswered,
    DurableStep::StoreWrite,
];

/// How many times the relaunched wallet's user tries to send again before the test gives up
const MAX_SEND_RETRIES: usize = 3;

#[tokio::test(flavor = "current_thread")]
async fn a_crash_at_any_step_of_a_payjoin_sends_one_transaction() {
    crash_at_every_step(Answer::Proposal, RecoveryFault::None).await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_crash_at_any_step_of_a_payjoin_survives_signing_being_unavailable_on_relaunch() {
    crash_at_every_step(Answer::Proposal, RecoveryFault::SigningUnavailable).await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_crash_at_any_step_of_a_payjoin_survives_a_refused_broadcast_on_relaunch() {
    crash_at_every_step(Answer::Proposal, RecoveryFault::BroadcastRefusedOnce).await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_crash_at_any_step_of_a_payjoin_survives_a_failed_write_on_relaunch() {
    crash_at_every_step(Answer::Proposal, RecoveryFault::WriteFailsOnce).await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_crash_at_any_step_of_a_rejected_payjoin_sends_the_original_once() {
    crash_at_every_step(Answer::Rejection, RecoveryFault::None).await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_crash_at_any_step_of_a_rejected_payjoin_survives_signing_being_unavailable_on_relaunch()
{
    crash_at_every_step(Answer::Rejection, RecoveryFault::SigningUnavailable).await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_crash_at_any_step_of_a_rejected_payjoin_survives_a_refused_broadcast_on_relaunch() {
    crash_at_every_step(Answer::Rejection, RecoveryFault::BroadcastRefusedOnce).await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_crash_at_any_step_of_a_rejected_payjoin_survives_a_failed_write_on_relaunch() {
    crash_at_every_step(Answer::Rejection, RecoveryFault::WriteFailsOnce).await;
}

/// Runs one send per durable step, stopping after that step, then one send nothing stops
async fn crash_at_every_step(answer: Answer, fault: RecoveryFault) {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let services = PayjoinServices::start(1).await;

    for stop_after in 1..=STEPS_OF_A_SEND.len() + 1 {
        CrashRun { answer, fault, stop_after }.check(&services).await;
    }
}

/// How the receiver answers the Original
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum Answer {
    Proposal,
    Rejection,
}

/// A failure the relaunched wallet meets while it recovers the session
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum RecoveryFault {
    None,
    /// The wallet's secret is unreadable while startup recovers the session, as a locked
    /// keychain right after launch would leave it
    SigningUnavailable,
    /// The node refuses the relaunched wallet's first broadcast
    BroadcastRefusedOnce,
    /// The relaunched wallet's first write to its session record fails
    WriteFailsOnce,
}

/// One send stopped after its `stop_after`th durable step, past the last step for a send that
/// nothing stops
#[derive(Debug, Clone, Copy)]
struct CrashRun {
    answer: Answer,
    fault: RecoveryFault,
    stop_after: usize,
}

impl fmt::Display for CrashRun {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self { answer, fault, stop_after } = self;
        write!(f, "answer {answer:?}, stopped after step {stop_after}, recovery fault {fault:?}")
    }
}

impl CrashRun {
    fn stops(&self) -> bool {
        self.stop_after <= STEPS_OF_A_SEND.len()
    }

    /// The transaction that should reach the node
    fn expected_kind(&self) -> PayjoinTransactionKind {
        match self.answer {
            Answer::Rejection => PayjoinTransactionKind::Fallback,

            // before the sender saves the directory's answer to its POST, a relaunch cannot tell
            // whether the receiver has the Original, and re-posting is not retry-safe
            Answer::Proposal if self.stop_after <= 2 => PayjoinTransactionKind::Fallback,

            // with the proposal received but not yet committed as the transaction to send, a
            // relaunch that cannot save that commitment sends the Original instead, which is
            // safe because the proposal never left the wallet
            Answer::Proposal
                if self.stop_after == 4 && self.fault == RecoveryFault::WriteFailsOnce =>
            {
                PayjoinTransactionKind::Fallback
            }

            Answer::Proposal => PayjoinTransactionKind::Proposal,
        }
    }

    async fn check(self, services: &PayjoinServices) {
        let mut fx = Fixture::signet();
        let wallet =
            import_mnemonic_with_target(fresh_mnemonic(), fx.network, WalletMode::Main).unwrap();
        let id = wallet.id.clone();
        fx.own(&id);

        let relay = TestRelay::forwarding_to(services, 0).await;
        let config = TestRelay::sender_config(&[&relay], SESSION_TIMEOUT);
        let switch = CrashSwitch::after(self.stop_after);
        switch.install_on(&fx.node);

        let mut first = WalletProcess::launch(&id, config.clone(), |db| switch.store(db));
        first.fund(&fx).await;

        let mut receiver = PayjoinReceiver::start(
            services,
            fx.node.chain(),
            fresh_receiver_wallet(fx.network),
            Amount::from_sat(50_000),
        )
        .await;
        let psbt = first.pay(&receiver).await;
        let receiver_coin = receiver.coin_outpoint();

        // the receiver answers whenever the Original reaches it, in either process's lifetime
        let answer = self.answer;
        let receiver_answers = async move {
            let original = receiver.wait_for_original().await;
            match answer {
                Answer::Proposal => drop(receiver.respond(original).await),
                Answer::Rejection => receiver.reject(original).await,
            }

            std::future::pending::<()>().await;
        };

        let recovered = async {
            first.until_stopped(&self, &switch).await;
            first.kill(&self).await;
            fx.node.clear_rules();

            let mut recovery = Recovery::install(self.fault, &fx, &id, &relay);
            let second = WalletProcess::launch(&id, config, |db| recovery.store(db));
            recovery.after_startup(&id, &second, &relay).await;
            let retries = second.until_settled(&self).await;

            (second, retries)
        };

        let (second, retries) = tokio::select! {
            () = receiver_answers => unreachable!("the receiver waits once it has answered"),
            recovered = recovered => recovered,
        };

        let sent = self.assert_one_transaction_sent(&fx, &psbt, receiver_coin);
        second.assert_session_finished(&self).await;
        second.kill(&self).await;
        self.assert_wallet_saved(&id, sent, &psbt);

        if self.fault == RecoveryFault::None {
            assert_eq!(retries, 0, "{self}: a clean relaunch finishes the session on its own");
        }

        if self.stops() {
            assert!(switch.has_stopped(), "{self}: the send took the step it was stopped after");
        } else {
            assert_eq!(switch.steps(), STEPS_OF_A_SEND, "{self}: the send took every step once");
        }
    }

    /// Checks the node got exactly one transaction spending the sender's coin, and that it is
    /// the one this run should end with
    ///
    /// The fake node accepts a second spend of the same coin, as a node holding both in its
    /// mempool for a moment would, so a proposal and an Original both reaching it would show
    /// here. Sending the same transaction twice is allowed, since a relaunch cannot know whether
    /// the node got it
    ///
    /// A proposal must spend the receiver's coin, so a transaction the sender rebuilt on its own
    /// counts as neither
    fn assert_one_transaction_sent(
        &self,
        fx: &Fixture,
        psbt: &Psbt,
        receiver_coin: OutPoint,
    ) -> Txid {
        let sender_coin = psbt.unsigned_tx.input[0].previous_output;
        let spends: BTreeSet<Txid> = fx
            .node
            .chain()
            .broadcasts()
            .iter()
            .filter(|tx| tx.input.iter().any(|input| input.previous_output == sender_coin))
            .map(Transaction::compute_txid)
            .collect();

        let [sent] = spends.iter().copied().collect::<Vec<_>>()[..] else {
            panic!("{self}: exactly one transaction spending the sender's coin, got {spends:?}");
        };

        let sent_tx = fx.node.chain().broadcasts().into_iter().find(|tx| tx.compute_txid() == sent);
        let spends_receiver_coin = sent_tx
            .expect("the node keeps the transaction it accepted")
            .input
            .iter()
            .any(|input| input.previous_output == receiver_coin);
        let kind = match (sent == psbt.unsigned_tx.compute_txid(), spends_receiver_coin) {
            (true, _) => PayjoinTransactionKind::Fallback,
            (false, true) => PayjoinTransactionKind::Proposal,
            (false, false) => panic!("{self}: {sent} is neither the Original nor a proposal"),
        };
        assert_eq!(kind, self.expected_kind(), "{self}: the node got the expected transaction");

        sent
    }

    /// Checks the wallet as the next launch would load it has saved the sent transaction and
    /// no longer counts the coin it spent
    fn assert_wallet_saved(&self, id: &WalletId, sent: Txid, psbt: &Psbt) {
        let sender_coin = psbt.unsigned_tx.input[0].previous_output;
        let wallet = Wallet::try_load_persisted(id.clone()).expect("the wallet loads");
        assert!(wallet.bdk.get_tx(sent).is_some(), "{self}: the wallet saved {sent}");
        assert!(
            wallet.bdk.list_unspent().all(|coin| coin.outpoint != sender_coin),
            "{self}: the wallet counts its paid coin as spent"
        );
    }
}

/// A wallet actor standing in for one launch of the app, with no manager or lifecycle
/// registration, so dropping it ends the actor without the shutdown a quitting app runs
struct WalletProcess {
    actor: Addr<WalletActor>,
    screen: ScreenUpdates,
}

impl WalletProcess {
    /// Opens the wallet from its stores, as a launch does, keeping its Payjoin record in the
    /// store `session_store` wraps around the wallet's database
    fn launch(
        id: &WalletId,
        config: PayjoinConfig,
        session_store: impl FnOnce(WalletDataDb) -> Arc<dyn PayjoinSessionStore>,
    ) -> Self {
        let wallet = Wallet::try_load_persisted(id.clone()).expect("the wallet loads");
        let db = WalletDataDb::new_or_existing(id.clone()).expect("the wallet data opens");
        let store = session_store(db.clone());

        let channel = ReconcileChannel::new(100);
        let screen = ScreenUpdates::listen_to(&channel);
        let metadata = Arc::new(RwLock::new(wallet.metadata.clone()));
        let snapshot = Arc::new(RwLock::new(WalletSnapshot::from_wallet(&wallet)));
        let scan_status = Arc::new(RwLock::new(WalletScanStatus::Idle));
        let actor = WalletActor::new_with_metadata_and_db(
            wallet,
            channel.sender().clone(),
            scan_status,
            snapshot,
            db,
            metadata,
        )
        .with_payjoin_config_for_test(config)
        .with_payjoin_store_for_test(store);

        Self { actor: spawn_actor(actor), screen }
    }

    /// Gives the wallet a confirmed 100,000 sat coin and waits for its scan to find it
    async fn fund(&mut self, fx: &Fixture) {
        let info = call!(self.actor.address_at(0)).await.expect("the wallet actor answers");
        let script = bitcoin::Address::from_str(&info.address.to_string())
            .expect("the wallet's address parses")
            .assume_checked()
            .script_pubkey();

        let funding = fx.node.chain().pay(&script, Amount::from_sat(100_000));
        fx.node.chain().confirm(funding);
        call!(self.actor.wallet_scan_and_notify(false)).await.expect("the wallet actor answers");
        self.screen.until_scan_settles().await;
    }

    /// Starts paying the receiver 40,000 sats with Payjoin, returning the Original PSBT
    async fn pay(&self, receiver: &PayjoinReceiver) -> Psbt {
        let uri =
            AddressWithNetwork::try_new(&receiver.pj_uri().to_string()).expect("pj URI parses");
        let endpoint =
            uri.payjoin.as_ref().expect("the URI carries a pj endpoint").endpoint.clone();
        let fee_rate = BdkFeeRate::from_sat_per_vb(2).expect("a valid fee rate");

        let recipient = Address::from(uri.address);
        let psbt = call!(self.actor.build_tx(Amount::from_sat(40_000), recipient, fee_rate))
            .await
            .expect("the wallet actor answers")
            .expect("the payment builds");

        // a stop in the first two steps fails the start, which the relaunch then recovers
        let mode = UnsignedPaymentMode::Payjoin { intent: PayjoinIntent::new(endpoint) };
        let _started = call!(self.actor.initiate_payment(psbt.clone(), mode))
            .await
            .expect("the wallet actor answers");

        psbt
    }

    /// Waits for the run's stop, or for a run nothing stops to complete its session
    async fn until_stopped(&mut self, run: &CrashRun, switch: &CrashSwitch) {
        if !run.stops() {
            self.screen
                .until("the session to complete", |message| {
                    matches!(message, Msg::PayjoinStatusChanged(PayjoinStatus::Completed { .. }))
                })
                .await;

            return;
        }

        let wait = loaded_host_wait(Duration::from_secs(30));
        tokio::time::timeout(wait, switch.until_stopped())
            .await
            .unwrap_or_else(|_| panic!("{run}: the send reaches the step it stops after"));
    }

    /// Ends the actor without its shutdown, as the process dying would
    async fn kill(self, run: &CrashRun) {
        let terminated = self.actor.termination();
        drop(self);

        let wait = loaded_host_wait(Duration::from_secs(10));
        tokio::time::timeout(wait, terminated).await.unwrap_or_else(|_| {
            panic!("{run}: the killed wallet actor ends once nothing holds it")
        });
    }

    /// Lets the wallet finish the session, trying to send again whenever it goes idle with the
    /// record still stored, as a user would, and returns how many tries that took
    async fn until_settled(&self, run: &CrashRun) -> usize {
        for retries in 0..=MAX_SEND_RETRIES {
            self.until_idle(run).await;
            if self.session_is_stored().await {
                // the gate either cleans up the record or resumes the session, and refuses
                // this send either way
                let _refused = call!(self.actor.check_payjoin_send_gate_for_test())
                    .await
                    .expect("the wallet actor answers");
                continue;
            }

            return retries;
        }

        panic!("{run}: the session record outlives {MAX_SEND_RETRIES} tries to send again");
    }

    async fn until_idle(&self, run: &CrashRun) {
        let deadline = tokio::time::Instant::now() + loaded_host_wait(Duration::from_secs(30));
        loop {
            let idle = call!(self.actor.payjoin_phase_is_idle_for_test())
                .await
                .expect("the wallet actor answers");
            if idle {
                return;
            }

            assert!(tokio::time::Instant::now() < deadline, "{run}: the session finishes");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn session_is_stored(&self) -> bool {
        call!(self.actor.payjoin_session_for_test())
            .await
            .expect("the wallet actor answers")
            .is_some()
    }

    async fn assert_session_finished(&self, run: &CrashRun) {
        assert!(!self.session_is_stored().await, "{run}: the session record is deleted");

        let gate = call!(self.actor.check_payjoin_send_gate_for_test())
            .await
            .expect("the wallet actor answers");
        assert!(gate.is_ok(), "{run}: the send gate lets a new payment start: {gate:?}");
    }
}

/// The relaunched wallet's [`RecoveryFault`], in place until the run ends
struct Recovery {
    fault: RecoveryFault,
    hidden_secret: Option<WalletSecret>,
    held_relay: Option<TestRelayBehavior>,
}

impl Recovery {
    fn install(fault: RecoveryFault, fx: &Fixture, id: &WalletId, relay: &TestRelay) -> Self {
        let mut recovery = Self { fault, hidden_secret: None, held_relay: None };
        match fault {
            RecoveryFault::None | RecoveryFault::WriteFailsOnce => {}

            RecoveryFault::SigningUnavailable => {
                let keychain = Keychain::global();
                let secret = keychain.get_wallet_secret(id).expect("the secret reads");
                recovery.hidden_secret = Some(secret.expect("a hot wallet has a secret"));
                assert!(keychain.delete_wallet_secret(id), "the secret is hidden");

                // a resumed session must not hear from the receiver before the secret is back,
                // or it could sign a fresh proposal while the secret is still hidden, so only
                // startup's own recovery meets the hidden secret
                recovery.held_relay = Some(relay.behavior());
                relay.set_behavior(TestRelayBehavior::Down);
            }

            RecoveryFault::BroadcastRefusedOnce => refuse_first_broadcast(fx),
        }

        recovery
    }

    fn store(&self, db: WalletDataDb) -> Arc<dyn PayjoinSessionStore> {
        match self.fault {
            RecoveryFault::WriteFailsOnce => FlakyStore::failing_once(db),
            _ => Arc::new(db),
        }
    }

    /// Ends a fault that only lasts through startup, once the wallet has answered a message and
    /// so finished its startup resume
    async fn after_startup(&mut self, id: &WalletId, process: &WalletProcess, relay: &TestRelay) {
        let _status =
            call!(process.actor.payjoin_status()).await.expect("the wallet actor answers");
        if let Some(secret) = self.hidden_secret.take() {
            Keychain::global().save_wallet_secret(id, secret).expect("the secret is restored");
        }

        if let Some(behavior) = self.held_relay.take() {
            relay.set_behavior(behavior);
        }
    }
}

/// Makes the node refuse the next broadcast, as a node restarting would
fn refuse_first_broadcast(fx: &Fixture) {
    let refused = AtomicBool::new(false);
    fx.node.on("").script(move |request| {
        let broadcast = request.protocol == Protocol::Esplora && request.name == "/tx";
        if broadcast && !refused.swap(true, Ordering::SeqCst) {
            return Scripted::Fail { status: 503, message: "the node is restarting".into() };
        }

        Scripted::Serve
    });
}
