use crate::fiat::FiatCurrency;
use std::sync::Arc;

use crate::fiat::client::PriceResponse;
use act_zero::call;
use cove_types::{amount::Amount, fees::FeeRateOptionWithTotalFee, psbt::Psbt, unit::BitcoinUnit};
use cove_util::format::NumberFormatter as _;
use tracing::{debug, trace};

use super::{
    BtcOnChangeHandler, DeferredSender, Error, FeeSelection, FiatOnChangeHandler, FiatOrBtc,
    Message, Result, RustSendFlowManager, SendFlowError, SetAmountFocusField, State, btc_on_change,
    fiat_on_change,
    state::{MaxSelection, SendFlowManagerState},
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
        let mut sender = self.reconciler.deferred_sender();
        self.state.lock().entering_btc_amount = new.clone();

        let state: State = self.state.clone().into();
        let me = self.clone();

        let needs_fee_rate_options_base = self.state.lock().fee_rate_options_base.is_none();
        if needs_fee_rate_options_base {
            cove_tokio::task::spawn(async move {
                me.get_and_update_base_fee_rate_options().await;
            });
        }

        let handler = BtcOnChangeHandler::new(state);
        let changes = handler.on_change(&old, &new);
        trace!("btc_on_change_handler changes: {changes:?}");

        let btc_on_change::Changeset {
            entering_amount_btc,
            clear_max_selected,
            amount_btc,
            amount_fiat,
        } = changes;

        if clear_max_selected {
            self.unset_max_selected(&mut sender);
        }

        if let Some(amount) = amount_btc {
            let current_amount_sats = self.state.lock().amount_sats;
            let amount_sats = amount.to_sat();
            {
                let mut state = self.state.lock();
                if current_amount_sats != Some(amount_sats) {
                    state.clear_warning_acknowledgements();
                }
                state.amount_sats = Some(amount_sats);
            }

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
        trace!("fiat_field_changed {old_value} --> {new_value}");
        if old_value == new_value {
            return None;
        }

        let mut sender = self.reconciler.deferred_sender();

        // update the state
        self.state.lock().entering_fiat_amount = new_value.clone();

        let prices = self.app.prices()?;
        let selected_currency = self.state.lock().selected_fiat_currency;
        let max_selected = self.state.lock().max_selected.as_ref().map(|max| *max.amount);

        let handler = FiatOnChangeHandler::new(prices, selected_currency, max_selected);
        let Ok(result) = handler.on_change(&old_value, &new_value) else {
            tracing::error!("unable to get fiat on change result");
            return None;
        };

        trace!("result: {result:?}, old_value: {old_value}, new_value: {new_value}");
        let fiat_on_change::Changeset {
            entering_fiat_amount,
            fiat_value,
            btc_amount,
            clear_max_selected,
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
            {
                let mut state = self.state.lock();
                if state.amount_sats != Some(btc_amount) {
                    state.clear_warning_acknowledgements();
                }
                state.amount_sats = Some(btc_amount);
            }
            sender.queue(Message::UpdateAmountSats(btc_amount));
            self.schedule_fee_rate_update();
        }

        if clear_max_selected {
            self.unset_max_selected(&mut sender);
        }

        Some(())
    }

    fn unset_max_selected(&self, sender: &mut DeferredSender) {
        unset_max_selected(&mut self.state.lock(), sender);
    }

    pub(crate) fn selected_fee_rate_changed(
        self: &Arc<Self>,
        fee_rate: Arc<FeeRateOptionWithTotalFee>,
    ) {
        debug!("selected_fee_rate_changed: {fee_rate:?}");
        let mut sender = self.reconciler.deferred_sender();
        if let Some(options) = self.fee_rate_options() {
            self.apply_fee_selection(FeeSelection::new(options, fee_rate), &mut sender);
        }

        if self.validate_amount_internal(false) && self.validate_address_internal(false) {
            self.state.lock().focus_field = None;
            sender.queue(Message::UpdateFocusField(None));
        }
    }

    /// When amount is changed, we will need to update the entering and fiat amounts
    pub(crate) fn handle_amount_changed(self: &Arc<Self>, amount: Amount) {
        debug!("handle_amount_changed: {amount:?}");

        let mut sender = self.reconciler.deferred_sender();
        let amount_changed = apply_amount(&mut self.state.lock(), amount, &mut sender);
        if amount_changed {
            self.schedule_fee_rate_update();
        }
    }

    pub(crate) fn handle_focus_field_changed(
        self: &Arc<Self>,
        old: Option<SetAmountFocusField>,
        new: Option<SetAmountFocusField>,
    ) {
        debug!("handle_focus_field_changed: {old:?} --> {new:?}");

        let mut sender = self.reconciler.deferred_sender();

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
                let entering_fiat_amount = fiat_entry_text(amount_fiat, currency);

                self.state.lock().entering_fiat_amount = entering_fiat_amount.clone();
                sender.queue(Message::UpdateEnteringFiatAmount(entering_fiat_amount));
            }

            let unit = self.state.lock().metadata.selected_unit;
            match (amount, unit) {
                (Some(amount), BitcoinUnit::Sat) => {
                    let entering_btc_amount = amount.as_sats().thousands_int();
                    self.set_and_send_entering_btc_amount(entering_btc_amount, &mut sender);
                }
                (Some(amount_sats), BitcoinUnit::Btc) => {
                    let entering_btc_amount = amount_sats.as_btc().thousands();
                    self.set_and_send_entering_btc_amount(entering_btc_amount, &mut sender);
                }
                _ => {}
            }
        }

        self.state.lock().focus_field = new;
        sender.queue(Message::UpdateFocusField(new));
    }

    pub(crate) async fn select_max_send_report_error(self: &Arc<Self>) {
        let result = self.select_max_send().await;
        self.report_max_send_error(result);
    }

    pub(crate) async fn refresh_max_send_report_error(self: &Arc<Self>) {
        let result = self.refresh_max_send().await;
        self.report_max_send_error(result);
    }

    pub(crate) fn report_max_send_error(&self, result: Result<()>) {
        match result {
            Ok(()) => {}
            Err(SendFlowError::SendBelowDustLimit) => {
                self.reconciler.send(Message::SetAlert(SendFlowError::SendBelowDustLimit.into()));
            }
            Err(error) => {
                let error = Error::unable_to_get_max_send(error);
                self.reconciler.send(Message::SetAlert(error.into()));
            }
        }
    }

    /// Selects Max at the selected fee rate, as when the user taps Max
    pub(crate) async fn select_max_send(self: &Arc<Self>) -> Result<()> {
        debug!("select_max_send");
        if self.drain_max_send(MaxTrigger::Tap).await?.is_none() {
            return Ok(());
        }

        let mut sender = self.reconciler.deferred_sender();
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

    /// Drains a selected Max again after its address or fee rate changed
    ///
    /// The user did not ask for this, so it leaves focus alone, and it is dropped when the
    /// user cleared Max, such as by typing an amount, while the drain was being built
    pub(crate) async fn refresh_max_send(self: &Arc<Self>) -> Result<()> {
        debug!("refresh_max_send");
        self.drain_max_send(MaxTrigger::Refresh).await?;
        Ok(())
    }

    /// Drains the wallet for Max until a drain still matches the address and fee rate when it
    /// finishes, and stores it, returning the amount unless a refresh was dropped
    pub(crate) async fn drain_max_send(
        self: &Arc<Self>,
        trigger: MaxTrigger,
    ) -> Result<Option<Arc<Amount>>> {
        // load the options first, so Max drains at the rate the selection settles on rather than
        // a fallback rate read before they arrived
        let has_fee_selection = self.state.lock().fee_selection.is_some();
        if !has_fee_selection {
            self.get_or_update_fee_rate_options().await;
        }

        let wallet_actor = self.wallet_actor();
        loop {
            let request = self.state.lock().max_send_request()?;
            if request.fee_rate.is_below_minimum() {
                return Err(SendFlowError::FeeRateBelowMinimum);
            }

            let drain = call!(
                wallet_actor.build_ephemeral_drain_tx(request.address.clone(), request.fee_rate)
            );

            let psbt: Psbt = drain.await.unwrap()?.into();
            let amount = Arc::new(psbt.output_total_amount());
            trace!("max drain: {amount:?} at {:?}", request.fee_rate);

            match self.commit_max_drain(trigger, MaxSelection { amount, request }) {
                MaxCommit::Applied(amount) => return Ok(Some(amount)),
                MaxCommit::Dropped => return Ok(None),

                // the address or fee rate changed while the drain was built
                MaxCommit::Stale => continue,
            }
        }
    }

    /// Stores a drained Max with the amount it sets, under one lock, if it still answers the
    /// current request, so no edit or newer drain can land between the check and the write
    pub(crate) fn commit_max_drain(
        self: &Arc<Self>,
        trigger: MaxTrigger,
        selection: MaxSelection,
    ) -> MaxCommit {
        let mut sender = self.reconciler.deferred_sender();
        let amount = selection.amount.clone();
        let (commit, needs_fee_update) = {
            let mut state = self.state.lock();
            if trigger == MaxTrigger::Refresh && state.max_selected.is_none() {
                return MaxCommit::Dropped;
            }

            if state.max_send_request().ok().as_ref() != Some(&selection.request) {
                return MaxCommit::Stale;
            }

            // typing at or above the old Max kept it selected, but the new Max can land above
            // what was typed, and then the typed amount stands, as if typed after it
            let typed_sats = state
                .max_selected
                .as_ref()
                .filter(|_| trigger == MaxTrigger::Refresh)
                .and_then(|max| state.amount_sats.filter(|sats| *sats != max.amount.to_sat()));

            if typed_sats.is_some_and(|sats| sats < selection.amount.to_sat()) {
                unset_max_selected(&mut state, &mut sender);
                (MaxCommit::Dropped, true)
            } else {
                state.max_selected = Some(selection);
                sender.queue(Message::SetMaxSelected(amount.clone()));
                let amount_changed = apply_amount(&mut state, *amount, &mut sender);
                (MaxCommit::Applied(amount), amount_changed)
            }
        };

        // a new amount needs its own fee total, and a Max cleared for a typed amount leaves a
        // total priced as a drain; replacing the fee task also aborts a drain estimate in flight
        if needs_fee_update {
            self.schedule_fee_rate_update();
        }

        commit
    }

    pub(crate) fn handle_selected_unit_changed(
        self: &Arc<Self>,
        old: BitcoinUnit,
        new: BitcoinUnit,
    ) {
        let mut sender = self.reconciler.deferred_sender();
        self.state.lock().metadata.selected_unit = new;

        sender.queue(Message::RefreshPresenters);

        if old == new {
            return;
        }

        // if its already empty clear everything
        let nothing_entered = {
            let state = self.state.lock();
            state.amount_sats.is_none() || state.entering_btc_amount.is_empty()
        };
        if nothing_entered {
            return self.clear_send_amount();
        }

        // if we are entering fiat, then we don't need to update the entering field
        if self.state.lock().metadata.fiat_or_btc == FiatOrBtc::Fiat {
            return;
        }

        let Some(amount_sats) = self.state.lock().amount_sats else {
            return;
        };

        let entering_btc_amount = btc_entry_text(Amount::from_sat(amount_sats), new);
        self.set_and_send_entering_btc_amount(entering_btc_amount, &mut sender);
    }

    pub(crate) fn handle_btc_or_fiat_changed(
        self: &Arc<Self>,
        _old_value: FiatOrBtc,
        new_value: FiatOrBtc,
    ) {
        let mut sender = self.reconciler.deferred_sender();
        self.state.lock().metadata.fiat_or_btc = new_value;

        sender.queue(Message::RefreshPresenters);

        let Some(amount_sats) = self.state.lock().amount_sats else {
            return;
        };

        match new_value {
            FiatOrBtc::Btc => {
                let unit = self.state.lock().metadata.selected_unit;
                let entering_btc_amount = btc_entry_text(Amount::from_sat(amount_sats), unit);
                self.set_and_send_entering_btc_amount(entering_btc_amount, &mut sender);
            }

            FiatOrBtc::Fiat => {
                let currency = self.state.lock().selected_fiat_currency;
                let fiat_amount = self.state.lock().amount_fiat.unwrap_or_default();
                let entering_fiat_amount = fiat_entry_text(fiat_amount, currency);
                self.set_and_send_entering_fiat_amount(entering_fiat_amount, &mut sender);
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

        let amount_fiat = fiat_value(Amount::from_sat(amount), btc_price_in_fiat);
        self.state.lock().amount_fiat = Some(amount_fiat);
        self.reconciler.send(Message::UpdateAmountFiat(amount_fiat));
    }

    pub(crate) fn set_and_send_entering_btc_amount(
        self: &Arc<Self>,
        new_entering_btc_amount: String,
        deferred_sender: &mut DeferredSender,
    ) {
        set_entering_btc_amount(&mut self.state.lock(), new_entering_btc_amount, deferred_sender);
    }

    pub(crate) fn set_and_send_entering_fiat_amount(
        self: &Arc<Self>,
        new_entering_fiat_amount: String,
        deferred_sender: &mut DeferredSender,
    ) {
        set_entering_fiat_amount(&mut self.state.lock(), new_entering_fiat_amount, deferred_sender);
    }
}

/// Who asked for a Max drain
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MaxTrigger {
    /// The user tapped Max, so the drain always lands
    Tap,

    /// The address or fee rate changed under a selected Max, so the drain lands only while Max
    /// is still selected
    Refresh,
}

/// What became of a finished Max drain
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum MaxCommit {
    /// Stored as the Max amount and the amount to send
    Applied(Arc<Amount>),

    /// The address or fee rate changed while it was built, so it must be drained again
    Stale,

    /// A refresh whose Max the user cleared or typed under meanwhile
    Dropped,
}

fn unset_max_selected(state: &mut SendFlowManagerState, sender: &mut DeferredSender) {
    if state.max_selected.take().is_some() {
        sender.queue(Message::UnsetMaxSelected);
    }
}

/// Sets the amount to send and the text and fiat value shown for it, queueing the updates and
/// returning whether the amount changed
fn apply_amount(
    state: &mut SendFlowManagerState,
    amount: Amount,
    sender: &mut DeferredSender,
) -> bool {
    let btc_price_in_fiat = state.btc_price_in_fiat;
    match state.metadata.fiat_or_btc {
        FiatOrBtc::Fiat => {
            if let Some(price) = btc_price_in_fiat {
                let text = fiat_entry_text(fiat_value(amount, price), state.selected_fiat_currency);
                set_entering_fiat_amount(state, text, sender);
            }
        }

        FiatOrBtc::Btc => {
            let text = btc_entry_text(amount, state.metadata.selected_unit);
            set_entering_btc_amount(state, text, sender);
        }
    }

    let amount_sats = amount.to_sat();
    let amount_changed = state.amount_sats != Some(amount_sats);
    if amount_changed {
        state.clear_warning_acknowledgements();
        state.amount_sats = Some(amount_sats);
        sender.queue(Message::UpdateAmountSats(amount_sats));
    }

    if let Some(price) = btc_price_in_fiat {
        let amount_fiat = fiat_value(amount, price);
        state.amount_fiat = Some(amount_fiat);
        sender.queue(Message::UpdateAmountFiat(amount_fiat));
    }

    amount_changed
}

fn set_entering_btc_amount(
    state: &mut SendFlowManagerState,
    entering_btc_amount: String,
    sender: &mut DeferredSender,
) {
    if state.entering_btc_amount != entering_btc_amount {
        state.entering_btc_amount = entering_btc_amount.clone();
        sender.queue(Message::UpdateEnteringBtcAmount(entering_btc_amount));
    }
}

fn set_entering_fiat_amount(
    state: &mut SendFlowManagerState,
    entering_fiat_amount: String,
    sender: &mut DeferredSender,
) {
    if state.entering_fiat_amount != entering_fiat_amount {
        state.entering_fiat_amount = entering_fiat_amount.clone();
        sender.queue(Message::UpdateEnteringFiatAmount(entering_fiat_amount));
    }
}

/// The text shown in the bitcoin amount field for a committed amount in the selected unit
fn btc_entry_text(amount: Amount, unit: BitcoinUnit) -> String {
    match unit {
        BitcoinUnit::Btc => amount.btc_string(),
        BitcoinUnit::Sat => amount.as_sats().thousands_int(),
    }
}

/// The text shown in the fiat amount field, symbol first
fn fiat_entry_text(amount_fiat: f64, currency: FiatCurrency) -> String {
    format!("{}{}", currency.symbol(), amount_fiat.thousands_fiat())
}

fn fiat_value(amount: Amount, btc_price_in_fiat: u64) -> f64 {
    amount.as_btc() * btc_price_in_fiat as f64
}
