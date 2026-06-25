use std::sync::Arc;

use crate::{fiat::client::PriceResponse, transaction::FeeRate, wallet::Address};
use act_zero::call;
use cove_types::{amount::Amount, fees::FeeRateOptionWithTotalFee, psbt::Psbt, unit::BitcoinUnit};
use cove_util::format::NumberFormatter as _;
use cove_util::result_ext::ResultExt as _;
use tracing::{debug, trace, warn};

use super::{
    BtcOnChangeHandler, DeferredSender, EnterMode, Error, FeeSelection, FiatOnChangeHandler,
    FiatOrBtc, Message, Result, RustSendFlowManager, SendFlowError, SetAmountFocusField, State,
    btc_on_change, fiat_on_change,
};

impl RustSendFlowManager {
    pub(crate) fn handle_btc_field_changed(
        self: Arc<Self>,
        old: String,
        new: String,
    ) -> Option<()> {
        trace!("btc_field_changed {old} --> {new}");
        if old == new {
            return None;
        }

        // update the state
        let mut sender = DeferredSender::new(self.reconciler.clone());
        self.state.lock().entering_btc_amount = new.clone();

        let state: State = self.state.clone().into();
        let me = self.clone();

        let needs_fee_rate_options_base = self.state.lock().fee_rate_options_base.is_none();
        if needs_fee_rate_options_base {
            cove_tokio::task::spawn(async move {
                me.get_and_update_base_fee_rate_options().await;
            });
        }

        let handler = BtcOnChangeHandler::new(state.clone());
        let changes = handler.on_change(&old, &new);
        debug!("btc_on_change_handler changes: {changes:?}");

        let btc_on_change::Changeset { entering_amount_btc, max_selected, amount_btc, amount_fiat } =
            changes;

        match max_selected {
            Some(Some(max)) => {
                let max = Arc::new(max);
                self.state.lock().max_selected = Some(max.clone());
                sender.queue(Message::SetMaxSelected(max));
            }
            Some(None) => {
                let was_max_selected = self.state.lock().max_selected.take().is_some();
                if was_max_selected {
                    sender.queue(Message::UnsetMaxSelected);
                }
            }
            None => {}
        }

        if let Some(amount) = amount_btc {
            let current_amount_sats = self.state.lock().amount_sats;
            let amount_sats = amount.to_sat();
            self.state.lock().amount_sats = Some(amount_sats);

            if current_amount_sats != Some(amount_sats) {
                sender.queue(Message::UpdateAmountSats(amount_sats));
                self.schedule_fee_rate_update();
            }
        }

        if let Some(amount) = amount_fiat {
            self.state.lock().amount_fiat = Some(amount);
            sender.queue(Message::UpdateAmountFiat(amount));
        }

        if let Some(entering_amount) = entering_amount_btc {
            self.set_and_send_entering_btc_amount(entering_amount, &mut sender);
        }

        Some(())
    }

    pub(crate) fn handle_fiat_field_changed(
        self: &Arc<Self>,
        old_value: String,
        new_value: String,
    ) -> Option<()> {
        debug!("fiat_field_changed {old_value} --> {new_value}");
        if old_value == new_value {
            return None;
        }

        let mut sender = DeferredSender::new(self.reconciler.clone());

        // update the state
        self.state.lock().entering_fiat_amount = new_value.clone();

        let prices = self.app.prices()?;
        let selected_currency = self.state.lock().selected_fiat_currency;
        let max_selected = self.state.lock().max_selected.as_deref().copied();

        let handler = FiatOnChangeHandler::new(prices, selected_currency, max_selected);
        let Ok(result) = handler.on_change(&old_value, &new_value) else {
            tracing::error!("unable to get fiat on change result");
            return None;
        };

        debug!("result: {result:?}, old_value: {old_value}, new_value: {new_value}");
        let fiat_on_change::Changeset {
            entering_fiat_amount,
            fiat_value,
            btc_amount,
            max_selected,
        } = result;

        if let Some(entering_fiat_amount) = entering_fiat_amount {
            self.state.lock().entering_fiat_amount = entering_fiat_amount.clone();
            sender.queue(Message::UpdateEnteringFiatAmount(entering_fiat_amount));
        }

        if let Some(amount_fiat) = fiat_value {
            self.state.lock().amount_fiat = Some(amount_fiat);
            sender.queue(Message::UpdateAmountFiat(amount_fiat));
        }

        if let Some(btc_amount) = btc_amount {
            let btc_amount = btc_amount.as_sats();
            self.state.lock().amount_sats = Some(btc_amount);
            sender.queue(Message::UpdateAmountSats(btc_amount));
            self.schedule_fee_rate_update();
        }

        if max_selected == Some(None) {
            let was_max_selected = self.state.lock().max_selected.take().is_some();
            if was_max_selected {
                sender.queue(Message::UnsetMaxSelected);
            }
        }

        Some(())
    }

    pub(crate) fn selected_fee_rate_changed(
        self: &Arc<Self>,
        fee_rate: Arc<FeeRateOptionWithTotalFee>,
    ) {
        debug!("selected_fee_rate_changed: {fee_rate:?}");
        let mut sender = DeferredSender::new(self.reconciler.clone());
        if let Some(options) = self.fee_rate_options() {
            let selection = FeeSelection::new(options, fee_rate.clone());
            self.state.lock().fee_selection = Some(selection.clone());
            sender.queue(Message::UpdateFeeSelection(selection));
        }

        // max was selected before, so we need to update it to match the new fee rate
        let max_selected = self.state.lock().max_selected.clone();
        if max_selected.is_some() {
            let me = self.clone();
            cove_tokio::task::spawn(async move { me.select_max_send_report_error().await });
        }

        if self.validate_amount_internal(false) && self.validate_address_internal(false) {
            self.state.lock().focus_field = None;
            sender.queue(Message::UpdateFocusField(None));
        }

        // if we are in coin control mode max mode, change the amount when fee changes
        let mode = self.state.lock().mode.clone();
        match mode {
            EnterMode::CoinControl(cc) if cc.is_max_selected => {
                if let Some(total_fee) = fee_rate.total_fee {
                    let max_amount = cc.max_send();
                    let amount = max_amount - total_fee;
                    self.handle_amount_changed(amount);
                }
            }
            _ => {}
        }

        self.validate_fee_percentage_internal(true);
    }

    /// When amount is changed, we will need to update the entering and fiat amounts
    pub(crate) fn handle_amount_changed(self: &Arc<Self>, amount: Amount) {
        debug!("handle_amount_changed: {amount:?}");

        let mut sender = DeferredSender::new(self.reconciler.clone());
        let (unit, fiat_or_btc, btc_price_in_fiat) = {
            let state = self.state.lock();

            let unit = state.metadata.selected_unit;
            let fiat_or_btc = state.metadata.fiat_or_btc;
            let btc_price_in_fiat = state.btc_price_in_fiat;

            (unit, fiat_or_btc, btc_price_in_fiat)
        };

        match fiat_or_btc {
            FiatOrBtc::Fiat => {
                if let Some(price) = btc_price_in_fiat {
                    let currency = self.state.lock().selected_fiat_currency;
                    let amount_fiat = amount.as_btc() * (price as f64);

                    let enterting_amount_fiat =
                        format!("{}{}", currency.symbol(), amount_fiat.thousands_fiat());

                    self.set_and_send_entering_fiat_amount(enterting_amount_fiat, &mut sender);
                }
            }

            FiatOrBtc::Btc => {
                let amount_string = match unit {
                    BitcoinUnit::Btc => amount.btc_string(),
                    BitcoinUnit::Sat => amount.as_sats().thousands_int(),
                };

                self.set_and_send_entering_btc_amount(amount_string, &mut sender);
            }
        }

        let old_amount_sats = self.state.lock().amount_sats;
        let amount_sats = amount.to_sat();
        self.state.lock().amount_sats = Some(amount_sats);

        if old_amount_sats != Some(amount_sats) {
            sender.queue(Message::UpdateAmountSats(amount_sats));
            self.schedule_fee_rate_update();
        }

        if let Some(price) = btc_price_in_fiat {
            let amount_fiat = amount.as_btc() * (price as f64);
            self.state.lock().amount_fiat = Some(amount_fiat);
            sender.queue(Message::UpdateAmountFiat(amount_fiat));
        }
    }

    pub(crate) fn handle_focus_field_changed(
        self: &Arc<Self>,
        old: Option<SetAmountFocusField>,
        new: Option<SetAmountFocusField>,
    ) {
        debug!("handle_focus_field_changed: {old:?} --> {new:?}");

        let mut sender = DeferredSender::new(self.reconciler.clone());

        // most likely the first load, so ignore for now let front end handle it
        if old.is_none() && new.is_some() && self.state.lock().focus_field.is_none() {
            return;
        }

        // make sure having no focus field is only possible is address and amount are valid
        if new.is_none() {
            // hacky way of finding out if this is the initial load
            let should_show_error = {
                let state = self.state.lock();
                state.address.is_some()
                    && state.amount_sats.is_some()
                    && state.amount_sats.unwrap_or_default() != 0
            };

            if !self.validate_amount_internal(should_show_error) {
                self.state.lock().focus_field = Some(SetAmountFocusField::Amount);
                sender.queue(Message::UpdateFocusField(Some(SetAmountFocusField::Amount)));
                return;
            }

            if !self.validate_address_internal(should_show_error) {
                self.state.lock().focus_field = Some(SetAmountFocusField::Address);
                sender.queue(Message::UpdateFocusField(Some(SetAmountFocusField::Address)));
                return;
            }
        }

        // format on blur
        if old == Some(SetAmountFocusField::Amount) {
            let amount = self.state.lock().amount_sats.map(Amount::from_sat);
            let amount_fiat = self.state.lock().amount_fiat;

            if let Some(amount_fiat) = amount_fiat {
                let currency = self.state.lock().selected_fiat_currency;
                let entering_fiat_amount =
                    format!("{}{}", currency.symbol(), amount_fiat.thousands_fiat());

                self.state.lock().entering_fiat_amount = entering_fiat_amount.clone();
                sender.queue(Message::UpdateEnteringFiatAmount(entering_fiat_amount));
            }

            let unit = self.state.lock().metadata.selected_unit;
            match (amount, unit) {
                (Some(amount), BitcoinUnit::Sat) => {
                    let entering_btc_amount = amount.as_sats().thousands_int().to_string();
                    self.set_and_send_entering_btc_amount(entering_btc_amount, &mut sender);
                }
                (Some(amount_sats), BitcoinUnit::Btc) => {
                    let entering_btc_amount = amount_sats.as_btc().thousands().to_string();
                    self.set_and_send_entering_btc_amount(entering_btc_amount, &mut sender);
                }
                _ => {}
            }
        }

        self.state.lock().focus_field = new;
        sender.queue(Message::UpdateFocusField(new));
    }

    pub(crate) async fn select_max_send_report_error(self: &Arc<Self>) {
        match self.select_max_send().await {
            Ok(()) => {}
            Err(error) => {
                let error = SendFlowError::UnableToGetMaxSend(error.to_string());
                self.reconciler.send(Message::SetAlert(error.into()));
            }
        }
    }

    pub(crate) async fn select_max_send(self: &Arc<Self>) -> Result<()> {
        debug!("select_max_send");
        let mut sender = DeferredSender::new(self.reconciler.clone());

        // access the mutex once
        let (address, fee_rate_options, selected_fee_rate, selected_fee_rate_base) = {
            let state = self.state.lock();

            let address = state.address.clone();
            let address_string = &state.entering_address;

            let address = address
                .map(Arc::unwrap_or_clone)
                .or_else(|| Address::from_string(address_string, state.metadata.network).ok())
                .or_else(|| state.first_address.clone().map(Arc::unwrap_or_clone));

            let selected_fee_rate_base = state.fee_rate_options_base.clone();
            let fee_rate_options =
                state.fee_selection.as_ref().map(|selection| selection.options.clone());
            let selected_fee_rate =
                state.fee_selection.as_ref().map(|selection| selection.selected.clone());
            let address = address.ok_or(Error::InvalidAddress(address_string.to_string()))?;

            (address, fee_rate_options, selected_fee_rate, selected_fee_rate_base)
        };

        if fee_rate_options.is_none() {
            self.get_or_update_fee_rate_options().await;
        }

        let wallet_actor = self.wallet_actor();

        // use the selected fee rate if we have have
        // or the medium base fee rate
        // or a default of 50 sat/vb
        let fee_rate = selected_fee_rate
            .map(|selected| selected.fee_rate)
            .or_else(|| selected_fee_rate_base.map(|base| base.medium.fee_rate));

        if fee_rate.is_none() {
            warn!("unable to get selected fee rate or base fee rate using default of 50 sat/vb");
        }

        let fee_rate = fee_rate.unwrap_or_else(|| FeeRate::from_sat_per_vb(50.0));
        let psbt: Psbt = call!(wallet_actor.build_ephemeral_drain_tx(address, fee_rate))
            .await
            .unwrap()
            .map_err_str(Error::UnableToGetMaxSend)?
            .into();

        let total = Arc::new(psbt.output_total_amount());
        trace!("psbt: {psbt:?}, total: {total:?}, fee_rate: {fee_rate:?}");

        self.state.lock().max_selected = Some(total.clone());
        sender.queue(Message::SetMaxSelected(total.clone()));
        self.handle_amount_changed(*total);

        let address_is_valid = self.state.lock().address.is_some();
        if address_is_valid {
            self.state.lock().focus_field = None;
            sender.queue(Message::UpdateFocusField(None));
        } else {
            self.state.lock().focus_field = Some(SetAmountFocusField::Address);
            sender.queue(Message::UpdateFocusField(Some(SetAmountFocusField::Address)));
        }

        Ok(())
    }

    pub(crate) fn handle_selected_unit_changed(
        self: &Arc<Self>,
        old: BitcoinUnit,
        new: BitcoinUnit,
    ) {
        let mut sender = DeferredSender::new(self.reconciler.clone());
        self.state.lock().metadata.selected_unit = new;

        sender.queue(Message::RefreshPresenters);

        if old == new {
            return;
        }

        // if its already empty clear everything
        {
            let state = self.state.lock();
            let amount_is_empty = state.amount_sats.is_none();
            let entering_btc_amount_is_empty = state.entering_btc_amount.is_empty();
            drop(state);

            if entering_btc_amount_is_empty || amount_is_empty {
                return self.clear_send_amount();
            }
        }

        // if we are entering fiat, then we don't need to update the entering field
        if self.state.lock().metadata.fiat_or_btc == FiatOrBtc::Fiat {
            return;
        }

        let Some(amount_sats) = self.state.lock().amount_sats else {
            return;
        };

        match new {
            BitcoinUnit::Btc => {
                let amount_string = Amount::from_sat(amount_sats).btc_string();
                self.set_and_send_entering_btc_amount(amount_string, &mut sender);
            }
            BitcoinUnit::Sat => {
                let amount_string = amount_sats.thousands_int();
                self.set_and_send_entering_btc_amount(amount_string, &mut sender);
            }
        }
    }

    pub(crate) fn handle_btc_or_fiat_changed(
        self: &Arc<Self>,
        _old_value: FiatOrBtc,
        new_value: FiatOrBtc,
    ) {
        let mut sender = DeferredSender::new(self.reconciler.clone());
        self.state.lock().metadata.fiat_or_btc = new_value;

        sender.queue(Message::RefreshPresenters);

        let Some(amount_sats) = self.state.lock().amount_sats else {
            return;
        };

        match new_value {
            FiatOrBtc::Btc => {
                let amount = Amount::from_sat(amount_sats);

                let amount_fmt = match self.state.lock().metadata.selected_unit {
                    BitcoinUnit::Btc => amount.btc_string(),
                    BitcoinUnit::Sat => amount.sats_string(),
                };

                self.set_and_send_entering_btc_amount(amount_fmt.clone(), &mut sender);
            }

            FiatOrBtc::Fiat => {
                let currency = self.state.lock().selected_fiat_currency;
                let fiat_amount = self.state.lock().amount_fiat.unwrap_or_default();
                let fiat_amount_fmt =
                    format!("{}{}", currency.symbol(), fiat_amount.thousands_fiat(),);

                self.set_and_send_entering_fiat_amount(fiat_amount_fmt.clone(), &mut sender);
            }
        }
    }

    pub(crate) fn handle_prices_changed(self: &Arc<Self>, prices: Arc<PriceResponse>) {
        let selected_currency = self.state.lock().selected_fiat_currency;
        let btc_price_in_fiat = prices.get_for_currency(selected_currency);

        self.state.lock().btc_price_in_fiat = Some(btc_price_in_fiat);

        let Some(amount) = self.state.lock().amount_sats else {
            return;
        };

        let amount_fiat = Amount::from_sat(amount).as_btc() * (btc_price_in_fiat as f64);
        self.state.lock().amount_fiat = Some(amount_fiat);
        self.reconciler.send(Message::UpdateAmountFiat(amount_fiat));
    }

    pub(crate) fn set_and_send_entering_btc_amount(
        self: &Arc<Self>,
        new_entering_btc_amount: String,
        deffered_sender: &mut DeferredSender,
    ) {
        let is_changed = {
            let mut state = self.state.lock();
            let current = std::mem::take(&mut state.entering_btc_amount);
            state.entering_btc_amount = new_entering_btc_amount.clone();
            current != new_entering_btc_amount
        };

        if is_changed {
            deffered_sender.queue(Message::UpdateEnteringBtcAmount(new_entering_btc_amount));
        }
    }

    pub(crate) fn set_and_send_entering_fiat_amount(
        self: &Arc<Self>,
        new_entering_fiat_amount: String,
        deferred_sender: &mut DeferredSender,
    ) {
        let is_changed = {
            let mut state = self.state.lock();
            let current = std::mem::take(&mut state.entering_fiat_amount);
            state.entering_fiat_amount = new_entering_fiat_amount.clone();
            current != new_entering_fiat_amount
        };

        if is_changed {
            deferred_sender.queue(Message::UpdateEnteringFiatAmount(new_entering_fiat_amount));
        }
    }
}
