use std::sync::Arc;

use cove_types::address::AddressWithNetwork;
use tracing::debug;

use super::{DeferredSender, Message, RustSendFlowManager, SendFlowError, SetAmountFocusField};

impl RustSendFlowManager {
    /// Called when the user types or pastes into the address field.
    /// Handles plain addresses and full bitcoin: URIs (extracts amount if present).
    pub(crate) fn handle_entering_address_changed(self: &Arc<Self>, address: String) {
        debug!("handle_entering_address_changed: {address}");

        let mut sender = DeferredSender::new(self.reconciler.clone());

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
            state.address = address.clone();
            state.payjoin_endpoint = payjoin_endpoint;
        }
        sender.queue(Message::UpdateAddress(address.clone()));

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
            state.amount_sats = None;
            state.amount_fiat = None;
        }

        let mut sender = DeferredSender::new(self.reconciler.clone());
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
        let mut sender = DeferredSender::new(self.reconciler.clone());
        {
            let mut state = self.state.lock();
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
        let mut sender = DeferredSender::new(self.reconciler.clone());

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
