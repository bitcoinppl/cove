use std::sync::Arc;

use cove_types::{
    amount::Amount,
    unit::BitcoinUnit,
    utxo::{Utxo, UtxoList},
};
use tracing::debug;

use super::{RustSendFlowManager, state::EnterMode};

impl RustSendFlowManager {
    pub(crate) fn handle_coin_control_amount_changed(self: &Arc<Self>, amount: f64) -> Option<()> {
        debug!("handle_coin_control_amount_changed: {amount}");

        let mut coin_control_mode = match self.state.lock().mode.clone() {
            EnterMode::CoinControl(coin_control_mode) => coin_control_mode,
            _ => return None,
        };

        let unit = self.state.lock().metadata.selected_unit;
        let amount = match unit {
            BitcoinUnit::Btc => Amount::from_btc(amount).ok()?,
            BitcoinUnit::Sat => Amount::from_sat(amount as u64),
        };
        let amount = amount.min(coin_control_mode.max_send());

        // if the amount we are selecting is within 1000 sats of the max send, then select the max send
        let max_send_without_fees =
            self.max_send_minus_fees().filter(|amount| amount.as_sats() > 0);
        let max_send_threshold =
            self.max_send_minus_fees_and_small_utxo().or(max_send_without_fees);
        if let Some(max_send_threshold) = max_send_threshold
            && amount >= max_send_threshold
        {
            debug!(
                "setting coin control to max amount close to max {} {}",
                amount.as_sats(),
                max_send_threshold.as_sats()
            );

            let max_send_amount =
                max_send_without_fees.unwrap_or_else(|| coin_control_mode.max_send());
            coin_control_mode.is_max_selected = true;

            self.state.lock().mode = EnterMode::CoinControl(coin_control_mode);
            self.handle_amount_changed(max_send_amount);
            return Some(());
        }

        {
            let mut state = self.state.lock();
            coin_control_mode.is_max_selected = false;
            state.mode = EnterMode::CoinControl(coin_control_mode);
        }

        self.handle_amount_changed(amount);

        Some(())
    }

    pub(crate) fn reconcile_coin_control_amount_for_selected_fee(self: &Arc<Self>) {
        let (mut coin_control_mode, amount_sats, total_fee_sats) = {
            let state = self.state.lock();
            let EnterMode::CoinControl(coin_control_mode) = state.mode.clone() else {
                return;
            };

            let total_fee_sats = state
                .fee_selection
                .as_ref()
                .and_then(|selection| {
                    selection.selected.total_fee.or(selection.options.medium.total_fee)
                })
                .map(|fee| fee.as_sats());

            (coin_control_mode, state.amount_sats, total_fee_sats)
        };

        let Some(total_fee_sats) = total_fee_sats else {
            return;
        };

        let max_send_sats = coin_control_mode.max_send().as_sats().saturating_sub(total_fee_sats);
        if coin_control_mode.is_max_selected {
            if amount_sats != Some(max_send_sats) {
                self.handle_amount_changed(Amount::from_sat(max_send_sats));
            }
            return;
        }

        let Some(amount_sats) = amount_sats else {
            return;
        };

        if amount_sats < max_send_sats {
            return;
        }

        coin_control_mode.is_max_selected = true;
        self.state.lock().mode = EnterMode::CoinControl(coin_control_mode);
        self.handle_amount_changed(Amount::from_sat(max_send_sats));
    }

    pub(crate) fn handle_coin_control_entered_amount_changed(
        self: &Arc<Self>,
        amount: String,
        _is_focused: bool,
    ) -> Option<()> {
        debug!("handle_coin_control_entered_amount_changed: {amount}");
        let amount = amount.chars().filter(|c| c.is_numeric() || *c == '.').collect::<String>();
        let amount_float = amount.parse::<f64>().ok()?;

        self.handle_coin_control_amount_changed(amount_float)
    }

    pub(crate) fn set_coin_control_mode(self: &Arc<Self>, utxos: Vec<Utxo>) {
        if utxos.is_empty() {
            return;
        }

        match self.state.lock().mode.clone() {
            // already in coin control mode with the same utxos, so do nothing
            EnterMode::CoinControl(cc) if cc.utxo_list.utxos == utxos => {
                return;
            }
            _ => {}
        }

        let utxo_list = Arc::new(UtxoList::from(utxos));
        let total_minus_fees = {
            let mut state = self.state.lock();
            let total_fee_sats = state
                .fee_selection
                .as_ref()
                .and_then(|selection| selection.selected.total_fee.map(|f| f.as_sats()));

            state.clear_warning_acknowledgements();
            state.mode = EnterMode::coin_control_max(utxo_list.clone());
            let total_minus_fees =
                utxo_list.total.as_sats().saturating_sub(total_fee_sats.unwrap_or(1000));

            Amount::from_sat(total_minus_fees)
        };

        let me = self.clone();
        cove_tokio::task::spawn(async move {
            me.get_or_update_fee_rate_options().await;
        });

        self.handle_amount_changed(total_minus_fees);
    }

    pub(crate) fn disable_coin_control_mode(self: &Arc<Self>) {
        if !self.state.lock().mode.is_coin_control() {
            debug!("coin control mode is already disabled");
            return;
        }

        {
            let mut state = self.state.lock();
            state.clear_warning_acknowledgements();
            state.mode = EnterMode::SetAmount;
        }

        let me = self.clone();
        cove_tokio::task::spawn(async move {
            me.get_or_update_fee_rate_options().await;
        });
    }
}
