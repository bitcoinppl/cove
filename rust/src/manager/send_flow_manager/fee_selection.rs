use std::sync::Arc;

use act_zero::call;
use cove_common::consts::LOW_SEND_WARNING_SATS;
use tracing::debug;

use crate::{fee_client::FEE_CLIENT, transaction::FeeRate, wallet::Address};

use cove_types::{
    amount::Amount,
    fees::{FeeRateOptionWithTotalFee, FeeRateOptions, FeeRateOptionsWithTotalFee, FeeSpeed},
};

use super::{
    Error, Message, Result, RustSendFlowManager, SendFlowError, SendFlowFeeDetailsError,
    SendFlowMaxSendError, state::EnterMode, state::FeeSelection,
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

        let total_fee = psbt.fee().map_err(SendFlowFeeDetailsError::from)?;

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
        let fee_response = FEE_CLIENT.fetch_and_get_fees().await.ok()?;
        let fees = Arc::new(FeeRateOptions::from(fee_response));

        {
            let mut state = self.state.lock();
            state.fee_rate_options_base = Some(fees.clone());
            state.has_base_fees = true;
        }

        Some(fees)
    }

    pub(crate) async fn get_or_update_fee_rate_options(self: &Arc<Self>) {
        debug!("get_or_update_fee_rate_options");

        let mut sender = self.reconciler.deferred_sender();

        let (address, amount_sats) = {
            let state = self.state.lock();
            let address = state.address.clone();
            let amount_sats = state.amount_sats;
            (address, amount_sats)
        };

        debug!("get_or_update_fee_rate_options: {address:?}, {amount_sats:?}");
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
            if first_address.is_none() {
                self.get_first_address().await;
            }

            match (address, first_address) {
                (Some(address), _) => address,
                (None, Some(first_address)) => first_address,
                _ => return,
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
        .await
        .unwrap();

        let mut fee_rate_options = match new_fee_rate_options {
            Ok(fee_rate_options) => fee_rate_options,
            Err(_) => return,
        };

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
        {
            let mut state = state.lock();
            if state.fee_selection.as_ref() != Some(&fee_selection) {
                state.clear_warning_acknowledgements();
            }
            state.fee_selection = Some(fee_selection.clone());
        }

        self.reconcile_coin_control_amount_for_selected_fee();

        sender.queue(Message::UpdateFeeSelection(fee_selection));
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
                    let error = Error::from(SendFlowMaxSendError::from(error));
                    self.reconciler.send_async(Message::SetAlert(error.into())).await;
                    return None;
                }
            };

        let total_fee = match psbt.fee().map_err(SendFlowFeeDetailsError::from) {
            Ok(total_fee) => total_fee,
            Err(error) => {
                let error = Error::from(SendFlowMaxSendError::from(error));
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
