use std::sync::Arc;

use cove_types::address::AddressWithNetwork;
use tracing::debug;

use super::{Message, RustSendFlowManager, SendFlowError, SetAmountFocusField};

impl RustSendFlowManager {
    /// Called when the user types or pastes into the address field.
    /// Handles plain addresses and full bitcoin: URIs (extracts amount if present).
    pub(crate) fn handle_entering_address_changed(self: &Arc<Self>, address: String) {
        debug!("handle_entering_address_changed: {address}");

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

        let payjoin_endpoint = parsed
            .as_ref()
            .and_then(|address_with_network| address_with_network.payjoin.as_ref())
            .map(|payjoin| payjoin.endpoint.clone());
        let address = parsed.map(|address_with_network| Arc::new(address_with_network.address));
        {
            let mut state = self.state.lock();
            if state.address != address {
                state.clear_warning_acknowledgements();
            }
            state.address = address.clone();
            state.payjoin_endpoint = payjoin_endpoint;
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
                me.select_max_send_report_error().await;
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
            state.payjoin_endpoint = None;
        }
        sender.queue(Message::UpdateAddress(None));

        self.state.lock().entering_address = String::new();
        sender.queue(Message::UpdateEnteringAddress(String::new()));
    }

    pub(crate) fn handle_scan_code_changed(
        self: &Arc<Self>,
        _old_value: String,
        new_value: String,
    ) {
        debug!("handle_scan_code_changed {new_value}");
        let mut sender = self.reconciler.deferred_sender();

        let network = self.state.lock().metadata.network;
        let address_with_network = {
            let new_value_moved = new_value;
            match AddressWithNetwork::try_new(&new_value_moved) {
                Ok(address_with_network) => address_with_network,
                Err(err) => {
                    let error = SendFlowError::from_address_error(err, new_value_moved);
                    return self.send_alert(error);
                }
            }
        };

        if !address_with_network.is_valid_for_network(network) {
            let error = SendFlowError::WrongNetwork {
                address: address_with_network.address.to_string(),
                valid_for: address_with_network.network,
                current: network,
            };
            return self.send_alert(error);
        }

        // set address
        let payjoin_endpoint =
            address_with_network.payjoin.as_ref().map(|payjoin| payjoin.endpoint.clone());
        let address = Arc::new(address_with_network.address);

        {
            let mut state = self.state.lock();
            if state.address.as_ref() != Some(&address) {
                state.clear_warning_acknowledgements();
            }
            state.address = Some(address.clone());
            state.payjoin_endpoint = payjoin_endpoint;
        }
        sender.queue(Message::UpdateAddress(Some(address.clone())));

        self.state.lock().entering_address = address.to_string();
        sender.queue(Message::UpdateEnteringAddress(address.to_string()));

        // handle amount if its present
        let mut should_show_amount_error = false;

        // set amount if its valid
        let is_coin_control = self.state.lock().mode.is_coin_control();
        if let Some(amount) = address_with_network.amount
            && !is_coin_control
        {
            let max_was_selected = self.state.lock().max_selected.take().is_some();
            if max_was_selected {
                sender.queue(Message::UnsetMaxSelected);
            }

            should_show_amount_error = true;
            self.handle_amount_changed(amount);
        }

        // if amount is invalid, go to amount field
        if !self.validate_amount_internal(should_show_amount_error) {
            let focus_field = SetAmountFocusField::Amount;
            self.state.lock().focus_field = Some(focus_field);
            sender.queue(Message::UpdateFocusField(Some(focus_field)));
        }

        // if both address and amount are valid, then clear the focus field
        if self.validate_amount_internal(false) && self.validate_address_internal(false) {
            self.state.lock().focus_field = None;
            sender.queue(Message::UpdateFocusField(None));
        }

        // the address or amount might have changed
        // lets update the fee rate options if its needed
        let me = self.clone();
        let is_max_selected = self.state.lock().max_selected.is_some();
        cove_tokio::task::spawn(async move {
            me.get_or_update_fee_rate_options().await;
            if is_max_selected {
                me.select_max_send_report_error().await;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use cove_tokio::FutureTimeoutExt as _;
    use cove_types::{address::AddressWithNetwork, fees::FeeRateOptions};

    use crate::{
        app::{App, AppAction},
        manager::wallet_manager::test_support::funded_preview_wallet_manager,
        router::{Route, SendConfirmationInput, SendRoute, SendRouteConfirmArgs},
        wallet::{balance::Balance, metadata::WalletMetadata},
    };

    use super::super::{Action, DebouncedTask, ReconcileChannel, RustSendFlowManager, State};

    const ADDRESS: &str = "bc1q00000002ltfnxz6lt9g655akfz0lm6k9wva2rm";
    const AMOUNT_SATS: u64 = 50_000;

    // a BIP 77 v2 endpoint from the payjoin crate's own session test vectors
    const PJ_V2_ENDPOINT: &str = "HTTPS://PAYJO.IN/TXJCGKTKXLUUZ%23EX1C4UC6ES-OH1QYPM5JXYNS754Y4R45QWE336QFX6ZR8DQGVQCULVZTV20TFVEYDMFQC-RK1Q0DJS3VVDXWQQTLQ8022QGXSX7ML9PHZ6EDSF6AKEWQG758JPS2EV";

    /// A send flow over a hot wallet holding one confirmed output, with fee rates already
    /// fetched so nothing in the test reaches the network
    fn funded_send_flow() -> Arc<RustSendFlowManager> {
        let wallet_manager = Arc::new(funded_preview_wallet_manager(&[100_000]));
        let state = State::new(WalletMetadata::preview_new(), Arc::new(Balance::default()));
        let manager = Arc::new(RustSendFlowManager {
            app: App::global().clone(),
            wallet_manager,
            state: state.into_inner(),
            reconciler: ReconcileChannel::new(100),
            fee_check_task: DebouncedTask::new("fee_check", Duration::from_millis(200)),
        });

        {
            let mut state = manager.state.lock();
            state.fee_rate_options_base = Some(Arc::new(FeeRateOptions::_ffi_preview_new()));
            state.unlocked_spendable_sats = Some(100_000);
        }

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

    fn pushed_confirm_route() -> Option<SendRouteConfirmArgs> {
        crate::app::test_support::routes().into_iter().find_map(|route| match route {
            Route::Send(SendRoute::Confirm(args)) => Some(args),
            _ => None,
        })
    }

    /// Payjoin URIs pasted into the address field become an ordinary one-recipient send: the
    /// field shows the bare address, the amount comes from the URI, and the confirm screen
    /// gets no payjoin endpoint, so both apps sign and broadcast it as a normal payment
    #[tokio::test(flavor = "current_thread")]
    async fn payjoin_uris_entered_as_the_address_become_an_ordinary_draft() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;
        crate::test_support::ensure_tokio_runtime();
        crate::test_support::init_test_keychain();
        crate::database::test_support::init_test_database();
        crate::app::reconcile::test_support::init_noop_updater();

        let uris = [
            ("pj v2", format!("bitcoin:{ADDRESS}?amount=0.0005&pj={PJ_V2_ENDPOINT}"), true),
            (
                "pj v2 with pjos",
                format!("bitcoin:{ADDRESS}?amount=0.0005&pj={PJ_V2_ENDPOINT}&pjos=0"),
                true,
            ),
            ("pjos without pj", format!("bitcoin:{ADDRESS}?amount=0.0005&pjos=0"), false),
            (
                "BIP 78 pj",
                format!("bitcoin:{ADDRESS}?amount=0.0005&pj=https://example.com/payjoin&pjos=0"),
                false,
            ),
        ];

        for (label, uri, is_payjoin_uri) in uris {
            // guards the fixture: the v2 URIs must really be read as payjoin requests
            let parsed = AddressWithNetwork::try_new(&uri).expect("URI parses");
            assert_eq!(parsed.payjoin.is_some(), is_payjoin_uri, "{label}");

            App::global().handle_action(AppAction::UpdateRoute { routes: Vec::new() });
            let manager = funded_send_flow();

            manager.clone().dispatch(Action::NotifyEnteringAddressChanged(uri.clone()));

            {
                let state = manager.state.lock();
                assert_eq!(state.entering_address, ADDRESS, "{label}: the field shows the address");
                assert_eq!(
                    state.address.as_deref().map(ToString::to_string).as_deref(),
                    Some(ADDRESS),
                    "{label}"
                );

                assert_eq!(
                    state.amount_sats,
                    Some(AMOUNT_SATS),
                    "{label}: the URI sets the amount"
                );
            }

            wait_for("the fee total for the entered address", || {
                manager.selected_fee_rate().is_some_and(|fee| fee.total_fee.is_some())
            })
            .await;

            manager.clone().dispatch(Action::FinalizeAndGoToNextScreen);
            wait_for("the confirm screen", || pushed_confirm_route().is_some()).await;

            let confirm = pushed_confirm_route().expect("confirm route was pushed");
            assert_eq!(confirm.payjoin_endpoint, None, "{label}: no payjoin session starts");
            assert_eq!(confirm.input, SendConfirmationInput::Unsigned, "{label}");
            assert_eq!(confirm.details.sending_to.to_string(), ADDRESS, "{label}");
            assert_eq!(confirm.details.sending_amount.as_sats(), AMOUNT_SATS, "{label}");

            let paid: Vec<_> = confirm
                .details
                .psbt
                .0
                .unsigned_tx
                .output
                .iter()
                .filter(|output| output.script_pubkey == confirm.details.sending_to.script_pubkey())
                .map(|output| output.value.to_sat())
                .collect();
            assert_eq!(paid, vec![AMOUNT_SATS], "{label}: one output pays the URI amount");
        }

        App::global().handle_action(AppAction::UpdateRoute { routes: Vec::new() });
    }
}
