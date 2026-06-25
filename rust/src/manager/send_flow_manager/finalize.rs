use std::sync::Arc;

use crate::{
    app::AppAction, manager::deferred_dispatch::DeferredDispatch, router::RouteFactory,
    wallet::metadata::WalletType,
};
use cove_types::amount::Amount;

use super::{EnterMode, Message, RustSendFlowManager, SendFlowAlertState, SendFlowError};

impl RustSendFlowManager {
    /// Create the PSBT and everything is valid go to the next screen
    pub(crate) fn finalize_and_go_to_next_screen(self: &Arc<Self>) {
        if !self.validate_amount_internal(true) || !self.validate_address_internal(true) {
            return;
        }

        let Some(amount_sats) = self.state.lock().amount_sats else {
            return self.send_alert(SendFlowError::InvalidNumber);
        };

        let amount = Amount::from_sat(amount_sats);

        let Some(address) = self.state.lock().address.clone() else {
            let invalid_address = self.state.lock().entering_address.clone();
            return self.send_alert(SendFlowError::InvalidAddress(invalid_address));
        };

        let Some(selected_fee_rate) = self.selected_fee_rate() else {
            return self.send_alert(SendFlowError::UnableToGetFeeRate);
        };

        let Some(total_fee) = selected_fee_rate.total_fee else {
            let me = self.clone();
            let address_for_psbt = address.as_ref().clone();
            let fee_rate = selected_fee_rate.fee_rate;
            cove_tokio::task::spawn(async move {
                me.get_or_update_fee_rate_options().await;

                if me.selected_fee_rate().and_then(|fee| fee.total_fee).is_none() {
                    if matches!(
                        me.build_psbt(Some(address_for_psbt), Some(amount), fee_rate).await,
                        Err(SendFlowError::SendBelowDustLimit)
                    ) {
                        return me.send_alert_async(SendFlowError::SendBelowDustLimit).await;
                    }

                    return me
                        .send_alert_async(SendFlowError::UnableToGetFeeDetails(
                            "selected fee total unavailable".to_string(),
                        ))
                        .await;
                }

                me.finalize_and_go_to_next_screen();
            });
            return;
        };

        if total_fee.as_sats() > amount_sats {
            return self.send_alert(SendFlowAlertState::General {
                title: "Fee Too High!".to_string(),
                message: "The fee is higher than the amount you are sending".to_string(),
            });
        }

        if let Some(warning) = self.pending_send_warning() {
            return self.reconciler.send(Message::SetAlert(warning));
        }

        self.reconciler.send(Message::UpdateFocusField(None));

        let (wallet_type, wallet_id, payjoin_endpoint) = {
            let state = self.state.lock();
            (state.metadata.wallet_type, state.metadata.id.clone(), state.payjoin_endpoint.clone())
        };

        let me = self.clone();
        let send_mode = self.state.lock().mode.clone();
        let manager = self.wallet_manager.clone();

        cove_tokio::task::spawn(async move {
            let confirm_details = match send_mode {
                EnterMode::SetAmount => {
                    manager.confirm_txn(amount, address, selected_fee_rate.fee_rate).await
                }
                EnterMode::CoinControl(coin_control) => {
                    let amount =
                        if coin_control.is_max_selected { coin_control.max_send() } else { amount };

                    manager
                        .confirm_manual_txn(
                            coin_control.outpoints(),
                            amount,
                            address,
                            selected_fee_rate.fee_rate,
                        )
                        .await
                }
            };

            let details = match confirm_details {
                Ok(details) => details,
                Err(error) => {
                    return me.send_alert_async(SendFlowError::from(error)).await;
                }
            };

            let details = Arc::new(details);

            // save the unsigned transaction if its a cold wallet
            if matches!(wallet_type, WalletType::Cold | WalletType::XpubOnly)
                && let Err(e) = manager.save_unsigned_transaction(details.clone())
            {
                let error = SendFlowError::UnableToSaveUnsignedTransaction(e.to_string());
                me.send_alert_async(error).await;
            }

            // update the route send the frontend to the proper next screen
            let next_route = match wallet_type {
                WalletType::Hot => {
                    RouteFactory::new().send_confirm(wallet_id, details, payjoin_endpoint)
                }
                WalletType::Cold | WalletType::XpubOnly => {
                    RouteFactory::new().send_hardware_export(wallet_id, details)
                }
                WalletType::WatchOnly => {
                    return me
                        .send_alert_async(SendFlowError::UnableToBuildTxn("watch only".to_string()))
                        .await;
                }
            };

            let mut deferred = DeferredDispatch::<AppAction>::new();
            deferred.queue(AppAction::PushRoute(next_route));
        });
    }
}
