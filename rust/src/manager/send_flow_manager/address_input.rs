use std::sync::Arc;

use cove_types::address::AddressWithNetwork;
use tracing::debug;

use super::{Message, RustSendFlowManager, SendFlowError, SetAmountFocusField};
use crate::manager::deferred_sender::DeferredSender;

impl RustSendFlowManager {
    /// Called when the user types or pastes into the address field.
    /// Handles plain addresses and full bitcoin: URIs (extracts amount if present).
    pub(crate) fn handle_entering_address_changed(self: &Arc<Self>, address: String) {
        // only the length, since a mistaken paste can be a recovery phrase
        debug!("handle_entering_address_changed: {} chars", address.len());

        // a platform echoing back the text rust set, such as the bare address it shows for a
        // payment request, is not an edit, and re-parsing it would drop the request's Payjoin
        // endpoint
        if self.state.lock().entering_address == address {
            return;
        }

        let mut sender = self.reconciler.deferred_sender();

        self.state.lock().entering_address = address.clone();

        let network = self.state.lock().metadata.network;
        let parsed = AddressWithNetwork::try_new(&address).ok();
        let parsed = parsed
            .filter(|address_with_network| address_with_network.is_valid_for_network(network));

        // if input was a URI, show just the address in the text field
        if let Some(address_with_network) = &parsed {
            let clean = address_with_network.address.to_string();
            if clean != address {
                self.state.lock().entering_address = clean.clone();
                sender.queue(Message::UpdateEnteringAddress(clean));
            }
        }

        let is_coin_control = self.state.lock().mode.is_coin_control();
        if let Some(amount) =
            parsed.as_ref().and_then(|address_with_network| address_with_network.amount)
            && !is_coin_control
        {
            let max_was_selected = self.state.lock().max_selected.take().is_some();
            if max_was_selected {
                sender.queue(Message::UnsetMaxSelected);
            }
            self.handle_amount_changed(amount);
        }

        let payjoin =
            parsed.as_ref().and_then(|address_with_network| address_with_network.payjoin.clone());
        let address = parsed.map(|address_with_network| Arc::new(address_with_network.address));
        {
            let mut state = self.state.lock();
            if state.address != address {
                state.clear_warning_acknowledgements();
            }
            state.address = address.clone();
            state.payjoin = payjoin;
        }
        sender.queue(Message::UpdateAddress(address));

        // if both address and amount are valid, then clear the focus field, if amount is invalid, then focus on amount
        if self.validate_address_internal(false) {
            let focus_field = if self.validate_amount_internal(false) {
                None
            } else {
                Some(SetAmountFocusField::Amount)
            };

            self.state.lock().focus_field = focus_field;
            sender.queue(Message::UpdateFocusField(focus_field));
        }

        // when we have a valid address, use that to get the fee rate options
        let me = self.clone();
        let is_max_selected = self.state.lock().max_selected.is_some();
        cove_tokio::task::spawn(async move {
            me.get_or_update_fee_rate_options().await;

            if is_max_selected {
                me.refresh_max_send_report_error().await;
            }
        });
    }

    pub(crate) fn clear_send_amount(self: &Arc<Self>) {
        {
            let mut state = self.state.lock();
            if state.amount_sats.is_some() {
                state.clear_warning_acknowledgements();
            }
            state.amount_sats = None;
            state.amount_fiat = None;
        }

        let mut sender = self.reconciler.deferred_sender();
        sender.queue(Message::UpdateAmountFiat(0.0));
        sender.queue(Message::UpdateAmountSats(0));
        self.schedule_fee_rate_update();

        // fiat
        let currency = self.state.lock().selected_fiat_currency;
        let entering_fiat_amount = currency.symbol().to_string();
        self.set_and_send_entering_fiat_amount(entering_fiat_amount, &mut sender);

        // btc
        self.set_and_send_entering_btc_amount(String::new(), &mut sender);

        let was_max_selected = self.state.lock().max_selected.take().is_some();
        if was_max_selected {
            sender.queue(Message::UnsetMaxSelected);
        }
    }

    pub(crate) fn clear_address(self: &Arc<Self>) {
        let mut sender = self.reconciler.deferred_sender();
        {
            let mut state = self.state.lock();
            if state.address.is_some() {
                state.clear_warning_acknowledgements();
            }
            state.address = None;
            state.payjoin = None;
        }
        sender.queue(Message::UpdateAddress(None));

        self.state.lock().entering_address = String::new();
        sender.queue(Message::UpdateEnteringAddress(String::new()));
    }

    /// Pay a payment request scanned in the send screen or passed in when it opened
    ///
    /// The request's amount is applied quietly, since the wallet's balance may still be loading
    /// when the screen opens, and an amount checked against it then would alert wrongly
    pub(crate) fn handle_payment_request(self: &Arc<Self>, request: &AddressWithNetwork) {
        if let Err(error) = self.check_request_network(request) {
            return self.send_alert(error);
        }

        let mut sender = self.reconciler.deferred_sender();
        self.apply_payment_request(request, &mut sender);

        // go to the amount field while it still needs one, and leave the fields once both are set
        if self.validate_address_internal(false) {
            let focus_field = if self.validate_amount_internal(false) {
                None
            } else {
                Some(SetAmountFocusField::Amount)
            };

            self.state.lock().focus_field = focus_field;
            sender.queue(Message::UpdateFocusField(focus_field));
        }

        // a new address changes the fee, and a new amount can change a Max send
        let me = self.clone();
        let is_max_selected = self.state.lock().max_selected.is_some();
        cove_tokio::task::spawn(async move {
            me.get_or_update_fee_rate_options().await;
            if is_max_selected {
                me.refresh_max_send_report_error().await;
            }
        });
    }

    fn check_request_network(&self, request: &AddressWithNetwork) -> Result<(), SendFlowError> {
        let network = self.state.lock().metadata.network;
        if request.is_valid_for_network(network) {
            return Ok(());
        }

        Err(SendFlowError::WrongNetwork {
            address: request.address.to_string(),
            valid_for: request.network,
            current: network,
        })
    }

    /// Take the request's address and Payjoin endpoint, and its amount unless coin control fixes
    /// the amount
    fn apply_payment_request(
        self: &Arc<Self>,
        request: &AddressWithNetwork,
        sender: &mut DeferredSender<Message>,
    ) {
        let address = Arc::new(request.address.clone());

        {
            let mut state = self.state.lock();
            if state.address.as_ref() != Some(&address) {
                state.clear_warning_acknowledgements();
            }
            state.address = Some(address.clone());
            state.payjoin = request.payjoin.clone();
            state.entering_address = address.to_string();
        }
        sender.queue(Message::UpdateAddress(Some(address.clone())));
        sender.queue(Message::UpdateEnteringAddress(address.to_string()));

        let is_coin_control = self.state.lock().mode.is_coin_control();
        let Some(amount) = request.amount.filter(|_| !is_coin_control) else {
            return;
        };

        let max_was_selected = self.state.lock().max_selected.take().is_some();
        if max_was_selected {
            sender.queue(Message::UnsetMaxSelected);
        }

        self.handle_amount_changed(amount);
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use cove_tokio::FutureTimeoutExt as _;
    use cove_types::address::AddressWithNetwork;

    use crate::{
        app::{App, AppAction},
        manager::{
            deferred_sender::SingleOrMany,
            wallet_manager::test_support::funded_preview_wallet_manager_with_metadata,
        },
        network::Network,
        payjoin_setting::test_support::PayjoinChoiceGuard,
        router::{
            Route, SendConfirmationInput, SendRoute, SendRouteConfirmArgs, UnsignedPaymentMode,
        },
        test_support::hardware_signing::external_address,
        wallet::metadata::WalletMetadata,
    };

    use super::super::{Action, Message, RustSendFlowManager};

    const MAINNET_ADDRESS: &str = "bc1q00000002ltfnxz6lt9g655akfz0lm6k9wva2rm";
    const P2PKH_MAINNET_ADDRESS: &str = "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa";
    const AMOUNT_SATS: u64 = 50_000;

    // a BIP 77 v2 endpoint from the payjoin crate's own session test vectors
    const PJ_V2_ENDPOINT: &str = "HTTPS://PAYJO.IN/TXJCGKTKXLUUZ%23EX1C4UC6ES-OH1QYPM5JXYNS754Y4R45QWE336QFX6ZR8DQGVQCULVZTV20TFVEYDMFQC-RK1Q0DJS3VVDXWQQTLQ8022QGXSX7ML9PHZ6EDSF6AKEWQG758JPS2EV";

    /// A send flow over a hot wallet on `network` holding one confirmed output, with fee rates
    /// already fetched so nothing in the test reaches the network
    fn funded_send_flow(network: Network) -> Arc<RustSendFlowManager> {
        let mut metadata = WalletMetadata::preview_new();
        metadata.network = network;

        let wallet_manager =
            Arc::new(funded_preview_wallet_manager_with_metadata(metadata, &[100_000]));

        let manager = RustSendFlowManager::with_preview_fees(wallet_manager);
        manager.state.lock().unlocked_spendable_sats = Some(100_000);

        manager
    }

    async fn wait_for(description: &str, mut done: impl FnMut() -> bool) {
        async {
            while !done() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
        .with_timeout(crate::test_support::loaded_host_wait(Duration::from_secs(2)))
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {description}"));
    }

    fn drain_reconcile_messages(manager: &RustSendFlowManager) -> Vec<Message> {
        let receiver = manager.reconciler.receiver();
        let mut messages = Vec::new();

        while let Ok(message) = receiver.try_recv() {
            match message {
                SingleOrMany::Single(message) => messages.push(message),
                SingleOrMany::Many(batch) => messages.extend(batch),
            }
        }

        messages
    }

    fn pushed_confirm_route() -> Option<SendRouteConfirmArgs> {
        crate::app::test_support::routes().into_iter().find_map(|route| match route {
            Route::Send(SendRoute::Confirm(args)) => Some(args),
            _ => None,
        })
    }

    /// Payjoin URIs pasted into the address field become a one-recipient send: the field shows
    /// the bare address, even after the app writes it back, and the amount comes from the URI.
    /// With Payjoin switched on, only a v2 endpoint reaches the confirm screen as a Payjoin; a
    /// BIP 78 endpoint or a stray pjos is paid as a normal payment
    #[tokio::test(flavor = "current_thread")]
    async fn payjoin_uris_entered_as_the_address_confirm_only_v2_as_a_payjoin() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;
        crate::test_support::ensure_tokio_runtime();
        crate::test_support::init_test_keychain();
        crate::database::test_support::init_test_database();
        crate::app::reconcile::test_support::init_noop_updater();

        let _choice = PayjoinChoiceGuard::set(true);

        let signet_address = external_address(bitcoin::Network::Signet).to_string();
        let wallets =
            [(Network::Bitcoin, MAINNET_ADDRESS.to_string()), (Network::Signet, signet_address)];

        for (network, address) in wallets {
            let uris = [
                ("pj v2", format!("bitcoin:{address}?amount=0.0005&pj={PJ_V2_ENDPOINT}"), true),
                (
                    "pj v2 with pjos",
                    format!("bitcoin:{address}?amount=0.0005&pj={PJ_V2_ENDPOINT}&pjos=0"),
                    true,
                ),
                ("pjos without pj", format!("bitcoin:{address}?amount=0.0005&pjos=0"), false),
                (
                    "BIP 78 pj",
                    format!(
                        "bitcoin:{address}?amount=0.0005&pj=https://example.com/payjoin&pjos=0"
                    ),
                    false,
                ),
            ];

            for (uri_label, uri, is_payjoin_uri) in uris {
                let label = format!("{uri_label} on {network:?}");

                // guards the fixture: the v2 URIs must really be read as payjoin requests
                let parsed = AddressWithNetwork::try_new(&uri).expect("URI parses");
                assert_eq!(parsed.payjoin.is_some(), is_payjoin_uri, "{label}");

                App::global().handle_action(AppAction::UpdateRoute { routes: Vec::new() });
                let manager = funded_send_flow(network);

                manager.clone().dispatch(Action::NotifyEnteringAddressChanged(uri.clone()));

                // iOS writes the bare address rust shows back through the field's binding, and
                // that echo must not drop the request
                manager.clone().dispatch(Action::NotifyEnteringAddressChanged(address.clone()));

                {
                    let state = manager.state.lock();
                    assert_eq!(
                        state.entering_address, address,
                        "{label}: the field shows the address"
                    );

                    assert_eq!(
                        state.address.as_deref().map(ToString::to_string),
                        Some(address.clone()),
                        "{label}"
                    );

                    assert_eq!(
                        state.amount_sats,
                        Some(AMOUNT_SATS),
                        "{label}: the URI sets the amount"
                    );
                }

                // the apps only learn the address and amount from these messages, and show Next
                // once they have them
                let mut sent = drain_reconcile_messages(&manager);
                let sent_address = sent.iter().any(|message| {
                    matches!(message, Message::UpdateAddress(Some(sent)) if sent.to_string() == address)
                });

                assert!(sent_address, "{label}: the apps were not sent the address: {sent:?}");
                assert!(
                    sent.contains(&Message::UpdateEnteringAddress(address.clone())),
                    "{label}: the apps' field was not set to the bare address: {sent:?}"
                );
                assert!(
                    sent.contains(&Message::UpdateAmountSats(AMOUNT_SATS)),
                    "{label}: the apps were not sent the URI's amount: {sent:?}"
                );

                // fee work runs on another task, so its message may already be in `sent`
                wait_for("the apps to be sent the fee total", || {
                    sent.extend(drain_reconcile_messages(&manager));
                    sent.iter().any(|message| {
                        matches!(message, Message::UpdateFeeSelection(selection) if selection.selected.total_fee.is_some())
                    })
                })
                .await;

                wait_for("the fee total for the entered address", || {
                    manager.selected_fee_rate().is_some_and(|fee| fee.total_fee.is_some())
                })
                .await;

                manager.clone().dispatch(Action::FinalizeAndGoToNextScreen);
                wait_for("the confirm screen", || pushed_confirm_route().is_some()).await;

                let confirm = pushed_confirm_route().expect("confirm route was pushed");
                let SendConfirmationInput::Unsigned { mode } = &confirm.input else {
                    panic!("{label}: a hot wallet confirms an unsigned payment");
                };

                match (mode, &parsed.payjoin) {
                    (UnsignedPaymentMode::Payjoin { intent }, Some(payjoin)) => {
                        assert_eq!(intent.endpoint, payjoin.endpoint, "{label}");
                    }
                    (UnsignedPaymentMode::Standard, None) => {}
                    (mode, _) => panic!("{label}: confirmed as {mode:?}"),
                }

                assert_eq!(confirm.details.sending_to.to_string(), address, "{label}");
                assert_eq!(confirm.details.sending_amount.as_sats(), AMOUNT_SATS, "{label}");

                let paid: Vec<_> = confirm
                    .details
                    .psbt
                    .0
                    .unsigned_tx
                    .output
                    .iter()
                    .filter(|output| {
                        output.script_pubkey == confirm.details.sending_to.script_pubkey()
                    })
                    .map(|output| output.value.to_sat())
                    .collect();

                assert_eq!(paid, vec![AMOUNT_SATS], "{label}: one output pays the URI amount");
            }
        }

        App::global().handle_action(AppAction::UpdateRoute { routes: Vec::new() });
    }

    /// A payment request scanned after an amount and address were typed keeps that amount,
    /// replaces the fee total for the typed address with one for its own, and confirms with its
    /// Payjoin endpoint
    #[tokio::test(flavor = "current_thread")]
    async fn a_scanned_request_without_an_amount_refreshes_the_fee_and_keeps_its_payjoin() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;
        crate::test_support::ensure_tokio_runtime();
        crate::test_support::init_test_keychain();
        crate::database::test_support::init_test_database();
        crate::app::reconcile::test_support::init_noop_updater();

        let _choice = PayjoinChoiceGuard::set(true);
        App::global().handle_action(AppAction::UpdateRoute { routes: Vec::new() });

        let manager = funded_send_flow(Network::Bitcoin);
        // set directly, since typing an amount schedules its own delayed fee refresh, which
        // would refresh the fee for the scanned address on the request's behalf
        manager.state.lock().amount_sats = Some(AMOUNT_SATS);
        manager.clone().dispatch(Action::NotifyEnteringAddressChanged(MAINNET_ADDRESS.into()));

        let mut fee_before = None;
        wait_for("the fee total for the typed address", || {
            fee_before = manager.selected_fee_rate().and_then(|fee| fee.total_fee);
            fee_before.is_some()
        })
        .await;

        // a P2PKH output is larger than the typed P2WPKH one, so its fee total differs
        let uri = format!("bitcoin:{P2PKH_MAINNET_ADDRESS}?pj={PJ_V2_ENDPOINT}");
        let request = AddressWithNetwork::try_new(&uri).expect("URI parses");
        let endpoint = request.payjoin.clone().expect("the URI carries a v2 endpoint").endpoint;
        manager.clone().dispatch(Action::SetPaymentRequest(Arc::new(request)));

        wait_for("the fee total for the scanned address", || {
            manager
                .selected_fee_rate()
                .and_then(|fee| fee.total_fee)
                .is_some_and(|fee| Some(fee) != fee_before)
        })
        .await;

        assert_eq!(manager.state.lock().entering_address, P2PKH_MAINNET_ADDRESS);
        assert_eq!(manager.state.lock().amount_sats, Some(AMOUNT_SATS), "the typed amount stays");

        manager.clone().dispatch(Action::FinalizeAndGoToNextScreen);
        wait_for("the confirm screen", || pushed_confirm_route().is_some()).await;

        let confirm = pushed_confirm_route().expect("confirm route was pushed");
        App::global().handle_action(AppAction::UpdateRoute { routes: Vec::new() });

        let SendConfirmationInput::Unsigned { mode } = confirm.input else {
            panic!("a hot wallet confirms an unsigned payment");
        };
        let UnsignedPaymentMode::Payjoin { intent } = mode else {
            panic!("the scanned request confirms as a Payjoin, got {mode:?}");
        };
        assert_eq!(intent.endpoint, endpoint);
        assert_eq!(confirm.details.sending_amount.as_sats(), AMOUNT_SATS);
    }
}
