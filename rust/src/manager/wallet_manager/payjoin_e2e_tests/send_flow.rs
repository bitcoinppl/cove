//! Pays a payment request through the send screen's own flow: the URI goes into the address
//! field, the send flow builds the payment and picks how the confirm screen sends it, and the
//! wallet sends it the way the confirm screen does

use std::sync::Arc;

use act_zero::call;
use bitcoin::Amount;
use cove_types::{address::AddressWithNetwork, psbt::Psbt};

use super::{
    Fixture, Msg, PayjoinStatus, PayjoinTransactionKind, RustWalletManager, SESSION_TIMEOUT,
    ScreenUpdates, fresh_mnemonic, fresh_receiver_wallet, only_broadcast, receive_script,
};
use crate::{
    app::{App, AppAction},
    manager::{
        import_wallet_manager::import_mnemonic_with_target,
        send_flow_manager::{RustSendFlowManager, SendFlowManagerAction},
    },
    router::{Route, SendConfirmationInput, SendRoute, SendRouteConfirmArgs, UnsignedPaymentMode},
    test_support::{
        fake_node::wait_until,
        payjoin::{PayjoinReceiver, PayjoinServices},
    },
    wallet::metadata::WalletMode,
};

/// A v2 pj URI entered in the send screen reaches the confirm screen as a Payjoin, and sending
/// it ends with the receiver's proposal as the one broadcast
#[tokio::test(flavor = "current_thread")]
async fn a_pj_uri_sent_from_the_send_screen_broadcasts_the_receivers_proposal() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let mut fx = Fixture::signet();
    let services = PayjoinServices::start(2).await;
    let mut send = SendFromScreen::open(&mut fx, &services).await;

    let confirm = send.confirm(&send.uri).await;
    let SendConfirmationInput::Unsigned { mode } = confirm.input else {
        panic!("a hot wallet confirms an unsigned payment");
    };
    let UnsignedPaymentMode::Payjoin { intent } = &mode else {
        panic!("a v2 pj URI confirms as a Payjoin, got {mode:?}");
    };
    assert_eq!(intent.endpoint, send.endpoint, "the session pays the URI's endpoint");

    send.pay(&confirm.details.psbt, mode.clone()).await;
    let original = send.receiver.wait_for_original().await;
    send.receiver.respond(original).await;

    send.screen
        .until("the session to complete", |message| {
            matches!(message, Msg::PayjoinStatusChanged(PayjoinStatus::Completed { .. }))
        })
        .await;

    let Some(PayjoinStatus::Completed { kind, .. }) = send.screen.payjoin_statuses().pop() else {
        unreachable!("the completed status was seen");
    };
    assert_eq!(kind, PayjoinTransactionKind::Proposal, "the session ends with the proposal");

    let broadcasts = only_broadcast(&fx);
    let receiver_coin = send.receiver.coin_outpoint();
    assert!(
        broadcasts[0].input.iter().any(|input| input.previous_output == receiver_coin),
        "the broadcast is the receiver's proposal, spending the receiver's coin"
    );
}

/// A BIP 78 v1 pj URI, which Cove's sender can't speak, reaches the confirm screen as an
/// ordinary payment, which the wallet broadcasts as built without starting a Payjoin session
#[tokio::test(flavor = "current_thread")]
async fn a_v1_pj_uri_sent_from_the_send_screen_broadcasts_the_payment_as_built() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let mut fx = Fixture::signet();
    let services = PayjoinServices::start(2).await;
    let send = SendFromScreen::open(&mut fx, &services).await;

    // the same receiver address and amount, behind a v1 endpoint instead of the v2 one
    let v1_uri = format!("bitcoin:{}?amount=0.0004&pj=https://example.com/payjoin", send.address);
    let parsed = AddressWithNetwork::try_new(&v1_uri).expect("the v1 URI parses");
    assert!(parsed.payjoin.is_none(), "a v1 endpoint is never read as a Payjoin request");

    let confirm = send.confirm(&v1_uri).await;
    let SendConfirmationInput::Unsigned { mode } = confirm.input else {
        panic!("a hot wallet confirms an unsigned payment");
    };
    assert_eq!(mode, UnsignedPaymentMode::Standard, "a v1 pj URI is paid as an ordinary payment");
    assert_eq!(confirm.details.sending_amount.as_sats(), 40_000, "the URI's amount is paid");

    send.pay(&confirm.details.psbt, mode).await;
    wait_until("the payment to reach the node", || !fx.node.chain().broadcasts().is_empty()).await;

    let broadcasts = only_broadcast(&fx);
    assert_eq!(
        broadcasts[0].compute_txid(),
        confirm.details.psbt.0.unsigned_tx.compute_txid(),
        "the node got the payment the confirm screen showed"
    );

    let session = call!(send.wallet.actor.payjoin_session_for_test())
        .await
        .expect("the wallet actor answers");
    assert!(session.is_none(), "no Payjoin session was started");
    assert!(send.screen.payjoin_statuses().is_empty(), "the screens saw no Payjoin status");
}

/// A funded hot wallet with its send screen open, and a receiver with a session open
struct SendFromScreen {
    wallet: Arc<RustWalletManager>,
    screen: ScreenUpdates,
    send_flow: Arc<RustSendFlowManager>,
    receiver: PayjoinReceiver,
    uri: String,
    address: String,
    endpoint: String,
}

impl SendFromScreen {
    /// The receiver asks for 40,000 sats of the wallet's 100,000 sat coin, so the payment has
    /// change
    async fn open(fx: &mut Fixture, services: &PayjoinServices) -> Self {
        let wallet =
            import_mnemonic_with_target(fresh_mnemonic(), fx.network, WalletMode::Main).unwrap();
        let (wallet, mut screen) = fx.open(wallet.id.clone());
        let funding =
            fx.node.chain().pay(&receive_script(&wallet, 0).await, Amount::from_sat(100_000));
        fx.node.chain().confirm(funding);
        wallet.start_wallet_scan().await.expect("the scan starts");
        screen.until_scan_settles().await;
        screen.clear();

        call!(wallet.actor.set_payjoin_config_for_test(services.sender_config(SESSION_TIMEOUT)))
            .await
            .expect("the wallet actor answers");

        let receiver = PayjoinReceiver::start(
            services,
            fx.node.chain(),
            fresh_receiver_wallet(fx.network),
            Amount::from_sat(50_000),
        )
        .await;

        let mut pj_uri = receiver.pj_uri();
        pj_uri.set_amount(Amount::from_sat(40_000));
        let uri = pj_uri.to_string();

        // guards the fixture: the receiver's URI must be read as a Payjoin request
        let parsed = AddressWithNetwork::try_new(&uri).expect("the pj URI parses");
        let endpoint = parsed.payjoin.expect("the URI carries a v2 pj endpoint").endpoint;
        let address = parsed.address.to_string();

        let wallet = Arc::new(wallet);
        let send_flow = RustSendFlowManager::with_preview_fees(wallet.clone());
        send_flow.load_wallet_for_test().await;

        Self { wallet, screen, send_flow, receiver, uri, address, endpoint }
    }

    /// Enters `uri` in the address field and presses Next, returning the confirm screen's route
    async fn confirm(&self, uri: &str) -> SendRouteConfirmArgs {
        App::global().handle_action(AppAction::UpdateRoute { routes: Vec::new() });

        let action = SendFlowManagerAction::NotifyEnteringAddressChanged(uri.to_string());
        self.send_flow.clone().dispatch(action);

        let send_flow = &self.send_flow;
        wait_until("the fee total for the entered payment", || {
            send_flow.selected_fee_rate().is_some_and(|fee| fee.total_fee.is_some())
        })
        .await;

        self.send_flow.clone().dispatch(SendFlowManagerAction::FinalizeAndGoToNextScreen);
        wait_until("the confirm screen", || pushed_confirm_route().is_some()).await;

        let confirm = pushed_confirm_route().expect("the confirm route was pushed");
        App::global().handle_action(AppAction::UpdateRoute { routes: Vec::new() });

        confirm
    }

    /// Sends the confirmed payment as the confirm screen's Send button does
    async fn pay(&self, psbt: &Psbt, mode: UnsignedPaymentMode) {
        self.wallet
            .initiate_payment(Arc::new(psbt.clone()), mode)
            .await
            .expect("the wallet sends the payment");
    }
}

fn pushed_confirm_route() -> Option<SendRouteConfirmArgs> {
    crate::app::test_support::routes().into_iter().find_map(|route| match route {
        Route::Send(SendRoute::Confirm(args)) => Some(args),
        _ => None,
    })
}
