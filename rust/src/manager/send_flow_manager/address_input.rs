use std::sync::Arc;

use cove_types::address::{Address, AddressWithNetwork};
use tracing::debug;

use super::{Message, RustSendFlowManager, SendFlowError, SetAmountFocusField};

impl RustSendFlowManager {
    /// Point the send at `address`; every change to the send's address goes through here
    ///
    /// A different address clears the warnings the user acknowledged for the old one and starts
    /// checking whether the new one is the user's own
    pub(crate) fn replace_address(
        self: &Arc<Self>,
        address: Option<Arc<Address>>,
        payjoin_endpoint: Option<String>,
    ) {
        let changed = {
            let mut state = self.state.lock();
            let changed = state.address != address;
            if changed {
                state.clear_warning_acknowledgements();
            }

            state.address = address.clone();
            state.payjoin_endpoint = payjoin_endpoint;
            changed
        };

        if let Some(address) = address.filter(|_| changed) {
            self.check_address_ownership(address);
        }
    }

    /// Called when the user types or pastes into the address field.
    /// Handles plain addresses and full bitcoin: URIs (extracts amount if present).
    pub(crate) fn handle_entering_address_changed(self: &Arc<Self>, address: String) {
        // only the length, since a mistaken paste can be a recovery phrase
        debug!("handle_entering_address_changed: {} chars", address.len());

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
        self.replace_address(address.clone(), payjoin_endpoint);
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
        self.replace_address(None, None);
        sender.queue(Message::UpdateAddress(None));

        self.state.lock().entering_address = String::new();
        sender.queue(Message::UpdateEnteringAddress(String::new()));
    }

    pub(crate) fn handle_scan_code_changed(
        self: &Arc<Self>,
        _old_value: String,
        new_value: String,
    ) {
        debug!("handle_scan_code_changed: {} chars", new_value.len());
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
        self.replace_address(Some(address.clone()), payjoin_endpoint);
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
                me.refresh_max_send_report_error().await;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use bdk_wallet::KeychainKind;
    use cove_tokio::FutureTimeoutExt as _;
    use cove_types::{
        address::{Address, AddressWithNetwork},
        fees::FeeRateOptions,
    };

    use crate::{
        app::{App, AppAction},
        manager::import_wallet_manager::import_mnemonic_with_target,
        manager::{
            deferred_sender::SingleOrMany,
            wallet_manager::test_support::funded_preview_wallet_manager_with_metadata,
        },
        network::Network,
        router::{Route, SendConfirmationInput, SendRoute, SendRouteConfirmArgs},
        test_support::{
            bip84_keys::{address_at, random_mnemonic},
            hardware_signing::external_address,
            wallet_target::WalletTargetGuard,
        },
        wallet::{
            balance::Balance,
            metadata::{WalletMetadata, WalletMode},
        },
    };

    use super::super::{
        Action, DebouncedTask, Message, ReconcileChannel, RustSendFlowManager, State,
        address_ownership::SendFlowAddressOwnership,
    };

    const MAINNET_ADDRESS: &str = "bc1q00000002ltfnxz6lt9g655akfz0lm6k9wva2rm";
    const AMOUNT_SATS: u64 = 50_000;

    // a BIP 77 v2 endpoint from the payjoin crate's own session test vectors
    const PJ_V2_ENDPOINT: &str = "HTTPS://PAYJO.IN/TXJCGKTKXLUUZ%23EX1C4UC6ES-OH1QYPM5JXYNS754Y4R45QWE336QFX6ZR8DQGVQCULVZTV20TFVEYDMFQC-RK1Q0DJS3VVDXWQQTLQ8022QGXSX7ML9PHZ6EDSF6AKEWQG758JPS2EV";

    /// A send flow over a hot wallet on `network` holding one confirmed output, with fee rates
    /// already fetched so nothing in the test reaches the network
    fn funded_send_flow(network: Network) -> Arc<RustSendFlowManager> {
        let mut metadata = WalletMetadata::preview_new();
        metadata.network = network;
        funded_send_flow_for(metadata)
    }

    /// A funded send flow, as in [`funded_send_flow`], sending from the wallet `metadata` names
    fn funded_send_flow_for(metadata: WalletMetadata) -> Arc<RustSendFlowManager> {
        let wallet_manager =
            Arc::new(funded_preview_wallet_manager_with_metadata(metadata.clone(), &[100_000]));

        let state = State::new(metadata, Arc::new(Balance::default()));
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
                assert_eq!(confirm.payjoin_endpoint, None, "{label}: no payjoin session starts");
                assert_eq!(confirm.input, SendConfirmationInput::Unsigned, "{label}");
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

    /// Typed, scanned and handed-in addresses are all checked against the user's wallets, and the
    /// apps get a note naming the wallet that owns the address, or nothing when none does
    #[tokio::test(flavor = "current_thread")]
    async fn the_apps_are_told_which_of_the_users_wallets_owns_the_address() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;
        crate::test_support::ensure_tokio_runtime();
        crate::test_support::init_test_keychain();
        crate::database::test_support::init_test_database();
        crate::app::reconcile::test_support::init_noop_updater();
        let _target = WalletTargetGuard::new(Network::Bitcoin, WalletMode::Main);

        let sending_keys = random_mnemonic();
        let sending =
            import_mnemonic_with_target(sending_keys.clone(), Network::Bitcoin, WalletMode::Main)
                .unwrap();
        let savings_keys = random_mnemonic();
        let savings =
            import_mnemonic_with_target(savings_keys.clone(), Network::Bitcoin, WalletMode::Main)
                .unwrap();

        let own = address_at(&sending_keys, Network::Bitcoin, KeychainKind::External, 3).address;
        let other = address_at(&savings_keys, Network::Bitcoin, KeychainKind::Internal, 2).address;

        // each owned address stays entered until its note arrives, as it would while the user
        // reads the screen; a note for an address that was already replaced is never sent
        let manager = funded_send_flow_for(sending);
        manager.clone().dispatch(Action::NotifyEnteringAddressChanged(MAINNET_ADDRESS.into()));
        manager
            .clone()
            .dispatch(Action::NotifyScanCodeChanged { old: String::new(), new: own.to_string() });

        // a note for the address no wallet owns would arrive here first
        assert_eq!(
            next_ownership_note(&manager).await,
            (own.to_string(), "This is an address in this wallet (receive, index 3)".to_string())
        );

        manager.clone().dispatch(Action::NotifyAddressChanged(Arc::new(other.clone())));
        assert_eq!(
            next_ownership_note(&manager).await,
            (
                other.to_string(),
                format!("This is your address in “{}” (change, index 2)", savings.name)
            )
        );
    }

    /// The address and message of the next ownership note sent to the apps
    async fn next_ownership_note(manager: &RustSendFlowManager) -> (String, String) {
        let mut note = None;
        wait_for("an address ownership note", || {
            note = note.take().or_else(|| {
                drain_reconcile_messages(manager).into_iter().find_map(|message| match message {
                    Message::UpdateAddressOwnership(note) => {
                        Some((note.address.to_string(), note.message))
                    }
                    _ => None,
                })
            });

            note.is_some()
        })
        .await;

        note.expect("a note arrived")
    }

    /// A lookup for an address the user has since replaced finishes after the new address's
    /// note, and must not send its note over the newer one
    #[test]
    fn a_note_for_a_replaced_address_is_never_sent() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        crate::test_support::ensure_tokio_runtime();
        crate::test_support::init_test_keychain();
        crate::database::test_support::init_test_database();
        crate::app::reconcile::test_support::init_noop_updater();

        let manager = funded_send_flow(Network::Bitcoin);
        let replaced = Arc::new(Address::preview_new());
        let current =
            Arc::new(AddressWithNetwork::try_new(MAINNET_ADDRESS).expect("address parses").address);
        manager.state.lock().address = Some(current.clone());

        let note = |address: &Arc<Address>| SendFlowAddressOwnership {
            address: address.clone(),
            message: address.to_string(),
        };
        manager.send_address_ownership(note(&current));
        manager.send_address_ownership(note(&replaced));

        let sent = drain_reconcile_messages(&manager);
        assert_eq!(sent, vec![Message::UpdateAddressOwnership(note(&current))]);
    }
}
