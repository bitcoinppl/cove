use std::sync::Arc;

use act_zero::call;
use cove_common::consts::LOW_SEND_WARNING_SATS;
use tracing::{info, trace, warn};

use crate::{fee_client::FEE_CLIENT, transaction::FeeRate, wallet::Address};

use cove_types::{
    amount::Amount,
    fees::{FeeRateOptionWithTotalFee, FeeRateOptions, FeeRateOptionsWithTotalFee, FeeSpeed},
};

use super::{
    DeferredSender, Error, Message, Result, RustSendFlowManager, SendFlowError,
    remote_fee_rate_options, state::EnterMode, state::FeeSelection,
};

fn selected_fee_rate_for_options(
    fee_options: &FeeRateOptionsWithTotalFee,
    selected_fee_rate: Option<&Arc<FeeRateOptionWithTotalFee>>,
) -> Arc<FeeRateOptionWithTotalFee> {
    let Some(selected_fee_rate) = selected_fee_rate else {
        return Arc::new(fee_options.medium);
    };

    match selected_fee_rate.fee_speed {
        FeeSpeed::Custom { .. } => {
            fee_options.custom().unwrap_or_else(|| fee_options.medium.into())
        }
        FeeSpeed::Fast => fee_options.fast.into(),
        FeeSpeed::Medium => fee_options.medium.into(),
        FeeSpeed::Slow => fee_options.slow.into(),
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl RustSendFlowManager {
    /// get the custom fee rate option
    #[uniffi::method]
    pub async fn get_custom_fee_option(
        self: &Arc<Self>,
        fee_rate: Arc<FeeRate>,
        fee_speed: FeeSpeed,
    ) -> Result<Arc<FeeRateOptionWithTotalFee>, Error> {
        let fee_rate = Arc::unwrap_or_clone(fee_rate);
        let psbt = self.build_psbt(None, None, fee_rate).await?;

        let total_fee = psbt.fee().map_err(Error::unable_to_get_fee_details)?;

        let fee_rate_option =
            FeeRateOptionWithTotalFee { fee_speed, fee_rate, total_fee: Some(total_fee) };

        Ok(fee_rate_option.into())
    }
}

impl RustSendFlowManager {
    pub(crate) fn selected_fee_rate(&self) -> Option<Arc<FeeRateOptionWithTotalFee>> {
        self.state.lock().fee_selection.as_ref().map(|selection| selection.selected.clone())
    }

    pub(crate) fn fee_rate_options(&self) -> Option<Arc<FeeRateOptionsWithTotalFee>> {
        self.state.lock().fee_selection.as_ref().map(|selection| selection.options.clone())
    }

    pub(crate) fn fee_selection_for_options(
        &self,
        fee_options: Arc<FeeRateOptionsWithTotalFee>,
    ) -> FeeSelection {
        let selected = self.selected_fee_rate();
        let selected = selected_fee_rate_for_options(&fee_options, selected.as_ref());
        FeeSelection::new(fee_options, selected)
    }

    pub(crate) async fn get_and_update_base_fee_rate_options(
        self: &Arc<Self>,
    ) -> Option<Arc<FeeRateOptions>> {
        let fee_response = match FEE_CLIENT.fetch_and_get_fees().await {
            Ok(fee_response) => fee_response,
            Err(error) => {
                self.record_fee_failure(Some(format!("unable to fetch fee rates: {error}")));
                return None;
            }
        };

        let fees = Arc::new(remote_fee_rate_options(fee_response)?);

        {
            let mut state = self.state.lock();
            state.fee_rate_options_base = Some(fees.clone());
            state.has_base_fees = true;
        }

        Some(fees)
    }

    /// Logs a fee update failure only when it differs from the last one, since fees recalculate
    /// on every amount or address edit
    pub(crate) fn record_fee_failure(&self, failure: Option<String>) {
        let previous = {
            let mut state = self.state.lock();
            if state.last_fee_failure == failure {
                return;
            }

            std::mem::replace(&mut state.last_fee_failure, failure.clone())
        };

        match (previous, failure) {
            (_, Some(failure)) => warn!("send flow fee update failed: {failure}"),
            (Some(_), None) => info!("send flow fee update recovered"),
            (None, None) => {}
        }
    }

    pub(crate) async fn get_or_update_fee_rate_options(self: &Arc<Self>) {
        trace!("get_or_update_fee_rate_options");

        let mut sender = self.reconciler.deferred_sender();

        let (address, amount_sats) = {
            let state = self.state.lock();
            let address = state.address.clone();
            let amount_sats = state.amount_sats;
            (address, amount_sats)
        };

        // the address and amount come from user input, and the amount can be a pasted PIN parsed
        // as sats, so only their presence is logged
        trace!(
            "get_or_update_fee_rate_options: has address {}, has amount {}",
            address.is_some(),
            amount_sats.is_some()
        );
        let wallet_actor = self.wallet_actor();
        let state = self.state.clone();

        let fee_rate_options_base = {
            let fee_rate_options_base = state.lock().fee_rate_options_base.clone();
            let fee_rate_options_base = match fee_rate_options_base {
                Some(fee_rate_options_base) => Some(fee_rate_options_base),
                None => self.get_and_update_base_fee_rate_options().await,
            };

            match fee_rate_options_base {
                Some(fee_rate_options_base) => Arc::unwrap_or_clone(fee_rate_options_base),
                None => return,
            }
        };

        let address = {
            let first_address = state.lock().first_address.clone();
            let first_address_result = match first_address {
                Some(_) => Ok(()),
                None => self.get_first_address().await,
            };

            match (address, first_address) {
                (Some(address), _) => address,
                (None, Some(first_address)) => first_address,
                (None, None) => {
                    // fees fall back to the first address until the user enters one
                    if let Err(error) = first_address_result {
                        self.record_fee_failure(Some(format!(
                            "unable to load first address: {error}"
                        )));
                    }

                    return;
                }
            }
        };

        let mode = self.state.lock().mode.clone();
        let address = Arc::unwrap_or_clone(address);

        let amount_sats_for_fee_calc = match &mode {
            EnterMode::CoinControl(cc) if cc.is_max_selected => cc.max_send().to_sat(),
            _ => amount_sats.unwrap_or(LOW_SEND_WARNING_SATS),
        };

        let amount_for_fee_calc = Amount::from_sat(amount_sats_for_fee_calc);
        let max_selected = self.state.lock().max_selected.clone();

        let new_fee_rate_options = match (max_selected, &mode) {
            (Some(_), _) => {
                call!(wallet_actor.fee_rate_options_with_total_fee_for_drain(
                    fee_rate_options_base,
                    address.clone()
                ))
            }
            (None, EnterMode::SetAmount) => {
                call!(wallet_actor.fee_rate_options_with_total_fee(
                    fee_rate_options_base,
                    amount_for_fee_calc.into(),
                    address.clone()
                ))
            }
            (None, EnterMode::CoinControl(cc)) => {
                call!(wallet_actor.fee_rate_options_with_total_fee_for_manual(
                    cc.utxo_list(),
                    fee_rate_options_base,
                    amount_for_fee_calc.into(),
                    address.clone()
                ))
            }
        }
        .await;

        let mut fee_rate_options = match new_fee_rate_options {
            Ok(Ok(fee_rate_options)) => fee_rate_options,
            Ok(Err(error)) => {
                return self.record_fee_failure(Some(format!("unable to calculate fees: {error}")));
            }

            // the wallet actor stops when the wallet closes, so this send flow is going away
            Err(_) => {
                return self.record_fee_failure(Some(
                    "wallet actor stopped during fee calculation".to_string(),
                ));
            }
        };

        self.record_fee_failure(None);

        let selected_fee_rate = self.selected_fee_rate();
        if let Some(updated_options) = self
            .updated_custom_fee_option(
                address.clone(),
                amount_for_fee_calc,
                fee_rate_options,
                selected_fee_rate.clone(),
            )
            .await
        {
            fee_rate_options = updated_options;
        }

        let fee_rate_options_with_total_fee = Arc::new(fee_rate_options);
        let selected = selected_fee_rate_for_options(
            &fee_rate_options_with_total_fee,
            selected_fee_rate.as_ref(),
        );

        let fee_selection = FeeSelection::new(fee_rate_options_with_total_fee, selected);
        self.apply_fee_selection(fee_selection, &mut sender);
    }

    /// Stores a fee selection and brings along everything derived from the selected rate
    ///
    /// Every write of the fee selection goes through here, whether the user picked a rate or a
    /// refresh replaced the options, so a selected Max always matches the rate it is sent at
    pub(crate) fn apply_fee_selection(
        self: &Arc<Self>,
        selection: FeeSelection,
        sender: &mut DeferredSender,
    ) {
        let selected = selection.selected.clone();
        let max_is_stale = {
            let mut state = self.state.lock();
            if state.fee_selection.as_ref() != Some(&selection) {
                state.clear_warning_acknowledgements();
            }

            state.fee_selection = Some(selection.clone());
            state.max_is_stale()
        };

        sender.queue(Message::UpdateFeeSelection(selection));

        // refreshed rates arrive without totals, and coin-control Max and the fee checks need one
        if selected.total_fee.is_none() {
            self.schedule_fee_rate_update();
        }

        self.reconcile_coin_control_amount_for_selected_fee();

        // a Max amount is drained at one rate, so at any other rate it leaves change or overspends;
        // a totals-only update keeps the rate, so it does not drain again
        if max_is_stale {
            let me = self.clone();
            cove_tokio::task::spawn(async move { me.refresh_max_send_report_error().await });
        }
    }

    /// Applies rates just fetched from the fee server, keeping the selected speed
    pub(crate) fn apply_fetched_fee_rates(self: &Arc<Self>, base_options: FeeRateOptions) {
        let fee_options = FeeRateOptionsWithTotalFee::without_totals(base_options);
        let selected = self
            .selected_fee_rate()
            .and_then(|selected| match selected.fee_speed {
                FeeSpeed::Fast => Some(Arc::new(fee_options.fast)),
                FeeSpeed::Medium => Some(Arc::new(fee_options.medium)),
                FeeSpeed::Slow => Some(Arc::new(fee_options.slow)),
                FeeSpeed::Custom { .. } => {
                    fee_options.get_fee_rate_with(selected.fee_rate.sat_per_vb())
                }
            })
            .unwrap_or_else(|| Arc::new(fee_options.medium));

        let fee_selection = FeeSelection::new(Arc::new(fee_options), selected);
        self.state.lock().fee_rate_options_base = Some(Arc::new(base_options));

        let mut sender = self.reconciler.deferred_sender();
        self.apply_fee_selection(fee_selection, &mut sender);

        // set last so Set Amount, which waits on it, opens with the refreshed selection in place
        self.state.lock().has_base_fees = true;
    }

    /// Returns the fee rate options with the updated custom fee
    async fn updated_custom_fee_option(
        self: &Arc<Self>,
        address: Address,
        amount: Amount,
        fee_rate_options: FeeRateOptionsWithTotalFee,
        selected_fee_rate: Option<Arc<FeeRateOptionWithTotalFee>>,
    ) -> Option<FeeRateOptionsWithTotalFee> {
        let selected_fee_rate = selected_fee_rate?;
        if !matches!(selected_fee_rate.fee_speed, FeeSpeed::Custom { .. }) {
            return None;
        }

        let psbt =
            match self.build_psbt(Some(address), Some(amount), selected_fee_rate.fee_rate).await {
                Ok(psbt) => psbt,
                Err(SendFlowError::SendBelowDustLimit) => {
                    self.reconciler
                        .send_async(Message::SetAlert(SendFlowError::SendBelowDustLimit.into()))
                        .await;
                    return None;
                }
                Err(error) => {
                    let error = Error::unable_to_get_max_send(error);
                    self.reconciler.send_async(Message::SetAlert(error.into())).await;
                    return None;
                }
            };

        let total_fee = match psbt.fee().map_err(Error::unable_to_get_fee_details) {
            Ok(total_fee) => total_fee,
            Err(error) => {
                let error = Error::unable_to_get_max_send(error);
                self.reconciler.send_async(Message::SetAlert(error.into())).await;
                return None;
            }
        };

        let mut new_custom_with_fee = Arc::unwrap_or_clone(selected_fee_rate.clone());
        new_custom_with_fee.total_fee = Some(total_fee);

        let fee_rate_options = fee_rate_options.add_custom_fee_rate(new_custom_with_fee.into());
        Some(fee_rate_options)
    }
}
