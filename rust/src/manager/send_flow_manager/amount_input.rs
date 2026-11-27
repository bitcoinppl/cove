use crate::fiat::FiatCurrency;
use std::sync::Arc;

use crate::{fiat::client::PriceResponse, manager::wallet_manager::WalletManagerError};
use act_zero::call;
use cove_types::{amount::Amount, fees::FeeRateOptionWithTotalFee, psbt::Psbt, unit::BitcoinUnit};
use cove_util::{ResultExt as _, format::NumberFormatter as _};
use tracing::{debug, trace};

use super::{
    BtcOnChangeHandler, DeferredSender, Error, FeeSelection, FiatOnChangeHandler, FiatOrBtc,
    Message, Result, RustSendFlowManager, SendFlowError, SetAmountFocusField, btc_on_change,
    fiat_on_change,
    state::{MaxSelection, SendFlowManagerState},
    validation::AmountAlert,
};

impl RustSendFlowManager {
    /// Applies the bitcoin amount field's new text, typed while the field showed `unit`
    ///
    /// The edit is checked against the entry mode, parsed and committed in one critical section,
    /// the same one a display preference change takes, so the text, the unit it is parsed in and
    /// the amount it commits always agree
    pub(crate) fn handle_btc_field_changed(self: Arc<Self>, new: String, unit: BitcoinUnit) {
        let (edit, needs_fee_rate_options_base) = self.update_state(|state, sender| {
            let edit = apply_btc_edit(state, new, unit, sender);
            (edit, state.fee_rate_options_base.is_none())
        });

        if edit == AmountEdit::Unchanged {
            return;
        }

        if needs_fee_rate_options_base {
            let me = self.clone();
            cove_tokio::task::spawn(async move {
                me.get_and_update_base_fee_rate_options().await;
            });
        }

        if edit == AmountEdit::Amount {
            self.schedule_fee_rate_update();
        }
    }

    /// Applies the fiat amount field's new text, checked, parsed and committed in one critical
    /// section like a bitcoin amount edit
    pub(crate) fn handle_fiat_field_changed(self: &Arc<Self>, new: String) {
        let prices = self.app.prices();
        let edit = self.update_state(|state, sender| apply_fiat_edit(state, new, prices, sender));

        if edit == AmountEdit::Amount {
            self.schedule_fee_rate_update();
        }
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

        if self.validate_amount_internal(AmountAlert::Silent)
            && self.validate_address_internal(false)
        {
            self.state.lock().focus_field = None;
            sender.queue(Message::UpdateFocusField(None));
        }
    }

    /// When amount is changed, we will need to update the entering and fiat amounts
    pub(crate) fn handle_amount_changed(self: &Arc<Self>, amount: Amount) {
        // the amount can be a pasted PIN parsed as sats, so it stays out of the log
        debug!("handle_amount_changed");

        let amount_changed = self.update_state(|state, sender| apply_amount(state, amount, sender));
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

            // leaving the fields doesn't ask to send, so a fee still loading isn't alerted
            let amount_alert = AmountAlert::shown_once_fee_loads_if(should_show_error);
            if !self.validate_amount_internal(amount_alert) {
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
            self.update_state(format_entered_amounts);
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

            let psbt: Psbt = drain.await.unwrap_or(Err(WalletManagerError::ActorNotFound))?.into();
            let amount = Arc::new(psbt.output_total_amount());
            let fee = psbt.fee().map_err_str(SendFlowError::UnableToGetFeeDetails)?;
            trace!("max drain: {amount:?} paying {fee:?} at {:?}", request.fee_rate);

            match self.commit_max_drain(trigger, MaxSelection { amount, fee, request }) {
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
        let amount = selection.amount.clone();
        let (commit, needs_fee_update) = self.update_state(|state, sender| {
            if trigger == MaxTrigger::Refresh && state.max_selected.is_none() {
                return (MaxCommit::Dropped, false);
            }

            if state.max_send_request().ok().as_ref() != Some(&selection.request) {
                return (MaxCommit::Stale, false);
            }

            // typing at or above the old Max kept it selected, but the new Max can land above
            // what was typed, and then the typed amount stands, as if typed after it
            let typed_sats = state
                .max_selected
                .as_ref()
                .filter(|_| trigger == MaxTrigger::Refresh)
                .and_then(|max| state.amount_sats.filter(|sats| *sats != max.amount.to_sat()));

            if typed_sats.is_some_and(|sats| sats < selection.amount.to_sat()) {
                unset_max_selected(state, sender);
                (MaxCommit::Dropped, true)
            } else {
                state.max_selected = Some(selection);
                sender.queue(Message::SetMaxSelected(amount.clone()));
                let amount_changed = apply_amount(state, *amount, sender);
                (MaxCommit::Applied(amount), amount_changed)
            }
        });

        // a new amount needs its own fee total, and a Max cleared for a typed amount leaves a
        // total priced as a drain; replacing the fee task also aborts a drain estimate in flight
        if needs_fee_update {
            self.schedule_fee_rate_update();
        }

        commit
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

/// What an amount field edit committed, for the work that runs once the state is unlocked
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AmountEdit {
    /// The text was the same as before, or an edit typed in an entry mode that has since changed
    /// set no amount, so nothing changed
    Unchanged,

    /// The text changed without setting an amount to send
    Text,

    /// The text set the amount to send, so its fees need recalculating
    Amount,
}

/// Parses the bitcoin field's new text, typed while the field showed `unit`, and commits it with
/// the amount and fiat value it sets
fn apply_btc_edit(
    state: &mut SendFlowManagerState,
    new: String,
    unit: BitcoinUnit,
    sender: &mut DeferredSender,
) -> AmountEdit {
    let mode = state.amount_entry.mode;
    if mode.fiat_or_btc != FiatOrBtc::Btc || mode.unit != unit {
        let changeset = BtcOnChangeHandler::typed_in(state, unit).parse_fresh(&new);
        let amount = changeset.amount_btc.map(|amount| (amount, changeset.clear_max_selected));

        return match amount {
            Some((amount, clear_max_selected)) => {
                commit_stale_edit(state, amount, clear_max_selected, sender)
            }
            None => {
                resend_field(&state.amount_entry.btc_text, sender, Message::UpdateEnteringBtcAmount)
            }
        };
    }

    let old = std::mem::replace(&mut state.amount_entry.btc_text, new.clone());

    // field text is raw user input and can hold a pasted PIN, so only its length is logged
    trace!("btc_field_changed len {} --> {}", old.len(), new.len());
    if old == new {
        return AmountEdit::Unchanged;
    }

    let btc_on_change::Changeset {
        entering_amount_btc,
        clear_max_selected,
        amount_btc,
        amount_fiat,
    } = BtcOnChangeHandler::new(state).on_change(&old, &new);

    if clear_max_selected {
        unset_max_selected(state, sender);
    }

    let mut edit = AmountEdit::Text;
    if let Some(amount) = amount_btc {
        let amount_sats = amount.to_sat();
        if state.amount_sats != Some(amount_sats) {
            state.clear_warning_acknowledgements();
            state.amount_sats = Some(amount_sats);
            sender.queue(Message::UpdateAmountSats(amount_sats));
            edit = AmountEdit::Amount;
        }
    }

    if let Some(amount) = amount_fiat {
        state.amount_fiat = Some(amount);
        sender.queue(Message::UpdateAmountFiat(amount));
    }

    if let Some(entering_amount) = entering_amount_btc {
        set_entering_btc_amount(state, entering_amount, sender);
    }

    edit
}

/// Parses the fiat field's new text at the given prices and commits it, with the amount it sets
///
/// With no prices the text can't be parsed, so it is kept as typed without an amount
fn apply_fiat_edit(
    state: &mut SendFlowManagerState,
    new: String,
    prices: Option<PriceResponse>,
    sender: &mut DeferredSender,
) -> AmountEdit {
    if state.amount_entry.mode.fiat_or_btc != FiatOrBtc::Fiat {
        let max_selected = state.max_selected.as_ref().map(|max| *max.amount);
        let currency = state.selected_fiat_currency;
        let amount = prices
            .and_then(|prices| {
                FiatOnChangeHandler::new(prices, currency, max_selected).on_change("", &new).ok()
            })
            .and_then(|changeset| Some((changeset.btc_amount?, changeset.clear_max_selected)));

        return match amount {
            Some((amount, clear_max_selected)) => {
                commit_stale_edit(state, amount, clear_max_selected, sender)
            }
            None => resend_field(
                &state.amount_entry.fiat_text,
                sender,
                Message::UpdateEnteringFiatAmount,
            ),
        };
    }

    let old = std::mem::replace(&mut state.amount_entry.fiat_text, new.clone());

    // field text is raw user input and can hold a pasted PIN, so only its length is logged
    trace!("fiat_field_changed len {} --> {}", old.len(), new.len());
    if old == new {
        return AmountEdit::Unchanged;
    }

    let Some(prices) = prices else { return AmountEdit::Text };

    let max_selected = state.max_selected.as_ref().map(|max| *max.amount);
    let handler = FiatOnChangeHandler::new(prices, state.selected_fiat_currency, max_selected);
    let Ok(changeset) = handler.on_change(&old, &new) else {
        tracing::error!("unable to get fiat on change result");
        return AmountEdit::Text;
    };

    let fiat_on_change::Changeset {
        entering_fiat_amount,
        fiat_value,
        btc_amount,
        clear_max_selected,
    } = changeset;

    if let Some(entering_fiat_amount) = entering_fiat_amount {
        state.amount_entry.fiat_text = entering_fiat_amount.clone();
        sender.queue(Message::UpdateEnteringFiatAmount(entering_fiat_amount));
    }

    if let Some(amount_fiat) = fiat_value {
        state.amount_fiat = Some(amount_fiat);
        sender.queue(Message::UpdateAmountFiat(amount_fiat));
    }

    let mut edit = AmountEdit::Text;
    if let Some(btc_amount) = btc_amount {
        let amount_sats = btc_amount.as_sats();
        if state.amount_sats != Some(amount_sats) {
            state.clear_warning_acknowledgements();
        }

        state.amount_sats = Some(amount_sats);
        sender.queue(Message::UpdateAmountSats(amount_sats));
        edit = AmountEdit::Amount;
    }

    if clear_max_selected {
        unset_max_selected(state, sender);
    }

    edit
}

/// Commits the amount of an edit typed before a display preference change moved the entry to
/// another unit or field
///
/// The change already rewrote the fields for the new mode, so the edit's text is not stored;
/// its amount is, parsed in the unit or field it was typed in and rendered in the current mode,
/// so the last keystroke before a switch still counts
fn commit_stale_edit(
    state: &mut SendFlowManagerState,
    amount: Amount,
    clear_max_selected: bool,
    sender: &mut DeferredSender,
) -> AmountEdit {
    debug!("committing an amount edit typed before the entry mode changed");
    if clear_max_selected {
        unset_max_selected(state, sender);
    }

    if apply_amount(state, amount, sender) { AmountEdit::Amount } else { AmountEdit::Text }
}

/// Drops an edit typed before a display preference change that sets no amount yet, such as
/// "0.", and sends the field's text again so the platform shows what Rust holds even when that
/// change left the text as it was
fn resend_field(
    text: &str,
    sender: &mut DeferredSender,
    message: fn(String) -> Message,
) -> AmountEdit {
    debug!("dropping a partial amount edit typed before the entry mode changed");
    sender.queue(message(text.to_string()));
    AmountEdit::Unchanged
}

/// Rewrites both amount fields from the committed amount once the amount field loses focus, in
/// one critical section so the bitcoin text is written in the unit the entry is in
fn format_entered_amounts(state: &mut SendFlowManagerState, sender: &mut DeferredSender) {
    if let Some(amount_fiat) = state.amount_fiat {
        let text = fiat_entry_text(amount_fiat, state.selected_fiat_currency);
        state.amount_entry.fiat_text.clone_from(&text);
        sender.queue(Message::UpdateEnteringFiatAmount(text));
    }

    let Some(amount) = state.amount_sats.map(Amount::from_sat) else { return };
    let text = match state.amount_entry.mode.unit {
        BitcoinUnit::Sat => amount.as_sats().thousands_int(),
        BitcoinUnit::Btc => amount.as_btc().thousands(),
    };

    set_entering_btc_amount(state, text, sender);
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
    match state.amount_entry.mode.fiat_or_btc {
        FiatOrBtc::Fiat => {
            if let Some(price) = btc_price_in_fiat {
                let text = fiat_entry_text(fiat_value(amount, price), state.selected_fiat_currency);
                set_entering_fiat_amount(state, text, sender);
            }
        }

        FiatOrBtc::Btc => {
            let text = btc_entry_text(amount, state.amount_entry.mode.unit);
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
    if state.amount_entry.btc_text != entering_btc_amount {
        state.amount_entry.btc_text = entering_btc_amount.clone();
        sender.queue(Message::UpdateEnteringBtcAmount(entering_btc_amount));
    }
}

fn set_entering_fiat_amount(
    state: &mut SendFlowManagerState,
    entering_fiat_amount: String,
    sender: &mut DeferredSender,
) {
    if state.amount_entry.fiat_text != entering_fiat_amount {
        state.amount_entry.fiat_text = entering_fiat_amount.clone();
        sender.queue(Message::UpdateEnteringFiatAmount(entering_fiat_amount));
    }
}

/// The text shown in the bitcoin amount field for a committed amount in the selected unit
pub(crate) fn btc_entry_text(amount: Amount, unit: BitcoinUnit) -> String {
    match unit {
        BitcoinUnit::Btc => amount.btc_string(),
        BitcoinUnit::Sat => amount.as_sats().thousands_int(),
    }
}

/// The text shown in the fiat amount field, symbol first
pub(crate) fn fiat_entry_text(amount_fiat: f64, currency: FiatCurrency) -> String {
    format!("{}{}", currency.symbol(), amount_fiat.thousands_fiat())
}

fn fiat_value(amount: Amount, btc_price_in_fiat: u64) -> f64 {
    amount.as_btc() * btc_price_in_fiat as f64
}
