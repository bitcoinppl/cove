use std::sync::Arc;

use crate::{
    app::AppAction,
    manager::deferred_dispatch::DeferredDispatch,
    router::RouteFactory,
    wallet::{Address, metadata::WalletType},
};
use cove_types::{amount::Amount, fees::FeeRateOptionWithTotalFee};

use super::{EnterMode, Message, RustSendFlowManager, SendFlowAlertState, SendFlowError};

#[derive(Clone)]
struct FinalizeSnapshot {
    amount_sats: u64,
    address: Arc<Address>,
    selected_fee_rate: Arc<FeeRateOptionWithTotalFee>,
    mode: EnterMode,
}

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
            let snapshot = FinalizeSnapshot {
                amount_sats,
                address: address.clone(),
                selected_fee_rate: selected_fee_rate.clone(),
                mode: self.state.lock().mode.clone(),
            };
            let address_for_psbt = address.as_ref().clone();
            let fee_rate = selected_fee_rate.fee_rate;
            cove_tokio::task::spawn(async move {
                me.get_or_update_fee_rate_options().await;

                if !me.finalize_snapshot_matches(&snapshot) {
                    return;
                }

                if me.selected_fee_rate().and_then(|fee| fee.total_fee).is_none() {
                    let psbt = me.build_psbt(Some(address_for_psbt), Some(amount), fee_rate).await;
                    if !me.finalize_snapshot_matches(&snapshot) {
                        return;
                    }

                    match psbt {
                        Ok(psbt) => {
                            let total_fee = match psbt.fee() {
                                Ok(total_fee) => total_fee,
                                Err(error) => {
                                    return me
                                        .send_alert_async(SendFlowError::UnableToGetFeeDetails(
                                            format!("selected fee total unavailable: {error}"),
                                        ))
                                        .await;
                                }
                            };
                            let mut selected_fee_rate =
                                Arc::unwrap_or_clone(snapshot.selected_fee_rate.clone());
                            selected_fee_rate.total_fee = Some(total_fee);

                            me.continue_with_selected_fee(
                                snapshot.amount_sats,
                                amount,
                                snapshot.address.clone(),
                                Arc::new(selected_fee_rate),
                                Some(total_fee),
                            );
                            return;
                        }
                        Err(
                            error @ (SendFlowError::SendBelowDustLimit
                            | SendFlowError::InsufficientFunds),
                        ) => {
                            return me.send_alert_async(error).await;
                        }
                        Err(error) => {
                            return me
                                .send_alert_async(SendFlowError::UnableToGetFeeDetails(format!(
                                    "selected fee total unavailable: {error}"
                                )))
                                .await;
                        }
                    }
                }

                me.finalize_and_go_to_next_screen();
            });
            return;
        };

        self.continue_with_selected_fee(
            amount_sats,
            amount,
            address,
            selected_fee_rate,
            Some(total_fee),
        );
    }

    fn continue_with_selected_fee(
        self: &Arc<Self>,
        amount_sats: u64,
        amount: Amount,
        address: Arc<Address>,
        selected_fee_rate: Arc<FeeRateOptionWithTotalFee>,
        total_fee: Option<Amount>,
    ) {
        if self.total_fee_blocks_send(amount_sats, total_fee) {
            return self.send_alert(SendFlowAlertState::General {
                title: "Fee Too High!".to_string(),
                message: "The fee is too high for the amount you are sending".to_string(),
            });
        }

        self.update_selected_fee_total_if_current(selected_fee_rate.clone());

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

    fn finalize_snapshot_matches(&self, snapshot: &FinalizeSnapshot) -> bool {
        let (amount_sats, address, selected_fee_rate, mode) = {
            let state = self.state.lock();
            let selected_fee_rate =
                state.fee_selection.as_ref().map(|selection| selection.selected.clone());

            (state.amount_sats, state.address.clone(), selected_fee_rate, state.mode.clone())
        };

        if amount_sats != Some(snapshot.amount_sats) {
            return false;
        }

        if address.as_deref() != Some(snapshot.address.as_ref()) {
            return false;
        }

        if mode != snapshot.mode {
            return false;
        }

        selected_fee_rate.as_deref().is_some_and(|current| {
            current.fee_speed == snapshot.selected_fee_rate.fee_speed
                && current.fee_rate == snapshot.selected_fee_rate.fee_rate
        })
    }

    fn total_fee_blocks_send(&self, amount_sats: u64, total_fee: Option<Amount>) -> bool {
        let Some(total_fee) = total_fee else {
            return false;
        };

        if total_fee.as_sats() > amount_sats {
            return true;
        }

        let mode = self.state.lock().mode.clone();
        let EnterMode::CoinControl(coin_control) = mode else {
            return false;
        };

        total_fee.as_sats() >= coin_control.max_send().as_sats()
    }

    fn update_selected_fee_total_if_current(
        self: &Arc<Self>,
        selected_fee_rate: Arc<FeeRateOptionWithTotalFee>,
    ) {
        let Some(total_fee) = selected_fee_rate.total_fee else {
            return;
        };

        let mut state = self.state.lock();
        let Some(selection) = &mut state.fee_selection else {
            return;
        };

        let current = &selection.selected;
        if current.fee_speed == selected_fee_rate.fee_speed
            && current.fee_rate == selected_fee_rate.fee_rate
            && current.total_fee != Some(total_fee)
        {
            selection.selected = selected_fee_rate;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use cove_types::fees::{
        FeeRateOption, FeeRateOptionWithTotalFee, FeeRateOptions, FeeRateOptionsWithTotalFee,
        FeeSpeed,
    };

    use crate::{
        manager::{deferred_sender::SingleOrMany, wallet_manager::RustWalletManager},
        wallet::{Address, balance::Balance, metadata::WalletMetadata},
    };

    use super::super::{
        App, DebouncedTask, FeeSelection, MessageSender, RustSendFlowManager, SendFlowAlertState,
        SendFlowWarningKind,
    };
    use super::*;

    fn manager_for_finalize() -> Arc<RustSendFlowManager> {
        crate::database::test_support::init_test_database();
        crate::test_support::ensure_tokio_runtime();

        let (sender, receiver) = flume::bounded(50);
        let balance = Arc::new(Balance::default());
        let state = super::super::State::new(WalletMetadata::preview_new(), balance);
        let wallet_manager = Arc::new(RustWalletManager::preview_new_wallet());

        Arc::new(RustSendFlowManager {
            app: App::global().clone(),
            wallet_manager,
            state: state.into_inner(),
            reconciler: MessageSender::new(sender),
            reconcile_receiver: Arc::new(receiver),
            fee_check_task: DebouncedTask::new("fee_check", Duration::from_millis(200)),
        })
    }

    fn selected_fee_without_total() -> FeeRateOptionWithTotalFee {
        FeeRateOptionWithTotalFee {
            fee_speed: FeeSpeed::Custom { duration_mins: 10 },
            fee_rate: FeeRateOption::new(FeeSpeed::Custom { duration_mins: 10 }, 1.0).fee_rate,
            total_fee: None,
        }
    }

    fn finalize_snapshot(
        address: Arc<Address>,
        selected_fee_rate: Arc<FeeRateOptionWithTotalFee>,
    ) -> FinalizeSnapshot {
        FinalizeSnapshot {
            amount_sats: 10_000,
            address,
            selected_fee_rate,
            mode: EnterMode::SetAmount,
        }
    }

    fn set_finalize_snapshot_state(
        manager: &RustSendFlowManager,
        address: Arc<Address>,
        selected_fee_rate: Arc<FeeRateOptionWithTotalFee>,
    ) {
        let options =
            FeeRateOptionsWithTotalFee::without_totals(FeeRateOptions::_ffi_preview_new());
        let mut state = manager.state.lock();
        state.amount_sats = Some(10_000);
        state.unlocked_spendable_sats = Some(50_000);
        state.address = Some(address);
        state.fee_selection = Some(FeeSelection::new(Arc::new(options), selected_fee_rate));
    }

    #[test]
    fn finalize_snapshot_allows_selected_fee_total_to_populate() {
        let manager = manager_for_finalize();
        let address = Arc::new(Address::preview_new());
        let selected_fee_rate = Arc::new(selected_fee_without_total());
        let snapshot = finalize_snapshot(address.clone(), selected_fee_rate.clone());
        set_finalize_snapshot_state(&manager, address, selected_fee_rate);

        let mut selected_with_total = selected_fee_without_total();
        selected_with_total.total_fee = Some(Amount::from_sat(2_000));
        manager.state.lock().fee_selection = Some(FeeSelection::new(
            Arc::new(
                FeeRateOptionsWithTotalFee::without_totals(FeeRateOptions::_ffi_preview_new()),
            ),
            Arc::new(selected_with_total),
        ));

        assert!(manager.finalize_snapshot_matches(&snapshot));
    }

    #[test]
    fn finalize_snapshot_rejects_changed_amount() {
        let manager = manager_for_finalize();
        let address = Arc::new(Address::preview_new());
        let selected_fee_rate = Arc::new(selected_fee_without_total());
        let snapshot = finalize_snapshot(address.clone(), selected_fee_rate.clone());
        set_finalize_snapshot_state(&manager, address, selected_fee_rate);

        manager.state.lock().amount_sats = Some(20_000);

        assert!(!manager.finalize_snapshot_matches(&snapshot));
    }

    #[test]
    fn finalize_snapshot_rejects_changed_fee_choice() {
        let manager = manager_for_finalize();
        let address = Arc::new(Address::preview_new());
        let selected_fee_rate = Arc::new(selected_fee_without_total());
        let snapshot = finalize_snapshot(address.clone(), selected_fee_rate.clone());
        set_finalize_snapshot_state(&manager, address, selected_fee_rate);

        let selected = Arc::new(FeeRateOptionWithTotalFee {
            fee_speed: FeeSpeed::Custom { duration_mins: 20 },
            fee_rate: FeeRateOption::new(FeeSpeed::Custom { duration_mins: 20 }, 2.0).fee_rate,
            total_fee: None,
        });
        let options =
            FeeRateOptionsWithTotalFee::without_totals(FeeRateOptions::_ffi_preview_new());
        manager.state.lock().fee_selection = Some(FeeSelection::new(Arc::new(options), selected));

        assert!(!manager.finalize_snapshot_matches(&snapshot));
    }

    #[test]
    fn fallback_total_fee_populates_selected_fee_before_warning_detection() {
        let manager = manager_for_finalize();
        let address = Arc::new(Address::preview_new());
        let selected = selected_fee_without_total();
        {
            let options =
                FeeRateOptionsWithTotalFee::without_totals(FeeRateOptions::_ffi_preview_new());
            let mut state = manager.state.lock();
            state.amount_sats = Some(10_000);
            state.unlocked_spendable_sats = Some(50_000);
            state.address = Some(address.clone());
            state.fee_selection = Some(FeeSelection::new(Arc::new(options), Arc::new(selected)));
        }

        let total_fee = Amount::from_sat(2_000);
        let mut selected = selected_fee_without_total();
        selected.total_fee = Some(total_fee);

        manager.continue_with_selected_fee(
            10_000,
            Amount::from_sat(10_000),
            address,
            Arc::new(selected),
            Some(total_fee),
        );

        let message = manager.reconcile_receiver.try_recv().expect("warning is reconciled");
        let SingleOrMany::Single(message) = message else {
            panic!("expected a single reconcile message");
        };

        assert!(matches!(
            message,
            super::super::SendFlowManagerReconcileMessage::SetAlert(SendFlowAlertState::Warning {
                kind: SendFlowWarningKind::VeryHighFee,
                ..
            })
        ));

        assert_eq!(manager.selected_fee_rate().and_then(|fee| fee.total_fee), Some(total_fee));
    }
}
