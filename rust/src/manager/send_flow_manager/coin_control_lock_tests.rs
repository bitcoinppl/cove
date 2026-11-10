//! Coin control cannot spend a UTXO that was locked after the send screen took its selection
//!
//! The send screen keeps the coin selection its route carried in, so a lock applied after
//! going back, from coin control or from transaction details, is not in that snapshot. Every
//! rebuild the send screen makes afterwards, for a fee change, an amount edit or Next, must
//! still refuse the locked coin, and continuing from coin control again must hand over a
//! selection without it

use std::{collections::HashSet, sync::Arc, time::Duration};

use bitcoin::OutPoint;
use cove_tokio::FutureTimeoutExt as _;
use cove_types::{
    amount::Amount,
    fees::{FeeRateOption, FeeRateOptions, FeeSpeed},
    psbt::Psbt,
    unit::BitcoinUnit,
    utxo::Utxo,
};

use super::{
    App, DebouncedTask, EnterMode, Message, ReconcileChannel, RustSendFlowManager,
    SendFlowAlertState, SendFlowError, State,
};

use crate::{
    manager::{
        coin_control_manager::{CoinControlManagerAction, RustCoinControlManager},
        deferred_sender::SingleOrMany,
        wallet_manager::WalletManagerError,
    },
    test_support::{
        hardware_signing::external_recipient,
        loaded_host_wait,
        utxo_locks::{LockTestWallet, OfflineChain},
    },
    transaction::{FeeRate, TxId},
    wallet::balance::Balance,
};

/// Three coins, each from its own receive transaction
struct Fixture {
    wallet: LockTestWallet,
    coin_control: Arc<RustCoinControlManager>,
    send: Arc<RustSendFlowManager>,
    to_lock: OutPoint,
    kept: OutPoint,
}

async fn coin_control_send_of_two_coins() -> Fixture {
    let mut chain = OfflineChain::new();
    let to_lock = chain.receive(&[60_000])[0];
    let kept = chain.receive(&[50_000])[0];
    chain.receive(&[40_000]);
    let wallet = chain.open();

    let coin_control =
        wallet.manager.new_coin_control_manager().await.expect("coin control manager opens");
    let send = send_flow(&wallet);

    // pick two coins in coin control and continue to the send screen
    coin_control.clone().dispatch(CoinControlManagerAction::NotifySelectedUtxosChanged(vec![
        Arc::new(to_lock.into()),
        Arc::new(kept.into()),
    ]));

    continue_from_coin_control(&coin_control, &send).await;

    assert_eq!(selected_outpoints(&send), HashSet::from([to_lock, kept]));
    assert!(send.state.lock().fee_selection.is_some(), "fee totals load for the selection");

    Fixture { wallet, coin_control, send, to_lock, kept }
}

/// The send flow `new` would build, without its network fee fetch: base fee rates are
/// already cached, so fee totals come from the wallet alone
fn send_flow(wallet: &LockTestWallet) -> Arc<RustSendFlowManager> {
    let metadata = wallet.manager.metadata.read().clone();
    let state = State::new(metadata, Arc::new(Balance::default()));

    {
        let mut state = state.lock();
        state.metadata.selected_unit = BitcoinUnit::Sat;
        state.fee_rate_options_base = Some(Arc::new(base_fee_rates()));
        state.has_base_fees = true;
        state.address = Some(Arc::new(external_recipient()));
    }

    Arc::new(RustSendFlowManager {
        app: App::global().clone(),
        wallet_manager: wallet.manager.clone(),
        state: state.into_inner(),
        reconciler: ReconcileChannel::new(50),
        fee_check_task: DebouncedTask::new("fee_check", Duration::from_millis(200)),
    })
}

fn base_fee_rates() -> FeeRateOptions {
    FeeRateOptions {
        fast: FeeRateOption::new(FeeSpeed::Fast, 10.0),
        medium: FeeRateOption::new(FeeSpeed::Medium, 5.0),
        slow: FeeRateOption::new(FeeSpeed::Slow, 2.0),
    }
}

/// What the coin control screen's Continue hands the send screen
async fn continue_from_coin_control(
    coin_control: &RustCoinControlManager,
    send: &Arc<RustSendFlowManager>,
) {
    let selected: Vec<Utxo> = coin_control.selected_utxos();
    send.set_coin_control_mode(selected);
    send.get_or_update_fee_rate_options().await;
}

fn selected_outpoints(send: &RustSendFlowManager) -> HashSet<OutPoint> {
    let EnterMode::CoinControl(coin_control) = send.state.lock().mode.clone() else {
        panic!("send screen is in coin control mode");
    };

    coin_control.outpoints().into_iter().collect()
}

fn coin_control_selection(coin_control: &RustCoinControlManager) -> HashSet<OutPoint> {
    coin_control
        .selected_utxos()
        .iter()
        .map(|utxo| OutPoint::from(utxo.outpoint.as_ref()))
        .collect()
}

fn psbt_inputs(psbt: &Psbt) -> HashSet<OutPoint> {
    psbt.0.unsigned_tx.input.iter().map(|input| input.previous_output).collect()
}

fn assert_rejects_locked_coin<T: std::fmt::Debug>(result: Result<T, SendFlowError>) {
    match result {
        Err(SendFlowError::WalletManager(WalletManagerError::LockedOutputsSelected)) => {}
        other => panic!("a selection holding a locked coin built anyway: {other:?}"),
    }
}

/// Taps Next and returns the alert it raises, which must come before any route change
async fn alert_after_next(send: &Arc<RustSendFlowManager>) -> SendFlowAlertState {
    let receiver = send.reconciler.receiver();
    while receiver.try_recv().is_ok() {}

    send.finalize_and_go_to_next_screen();

    async {
        loop {
            let messages = match receiver.recv_async().await.expect("reconciler stays open") {
                SingleOrMany::Single(message) => vec![message],
                SingleOrMany::Many(messages) => messages,
            };

            if let Some(alert) = messages.into_iter().find_map(|message| match message {
                Message::SetAlert(alert) => Some(alert),
                _ => None,
            }) {
                return alert;
            }
        }
    }
    .with_timeout(loaded_host_wait(Duration::from_secs(5)))
    .await
    .expect("Next raises an alert")
}

fn assert_locked_coin_alert(alert: SendFlowAlertState) {
    assert_eq!(
        alert,
        SendFlowAlertState::Error(SendFlowError::WalletManager(
            WalletManagerError::LockedOutputsSelected
        ))
    );
}

#[tokio::test(flavor = "current_thread")]
async fn coin_control_send_refuses_a_coin_locked_after_going_back_and_changing_the_fee() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let Fixture { wallet: _wallet, coin_control, send, to_lock, kept } =
        coin_control_send_of_two_coins().await;

    // back in coin control, lock one of the selected coins; it leaves the selection
    coin_control
        .set_utxo_spendability(Arc::new(to_lock.into()), false)
        .await
        .expect("coin control locks the coin");
    assert_eq!(coin_control_selection(&coin_control), HashSet::from([kept]));

    // the send screen still holds the selection from before the lock
    assert_eq!(selected_outpoints(&send), HashSet::from([to_lock, kept]));

    // switching to Fast and refreshing its total rebuilds with the locked coin
    let options = send.fee_rate_options().expect("fee options are loaded");
    send.selected_fee_rate_changed(Arc::new(options.fast));
    send.get_or_update_fee_rate_options().await;
    assert_rejects_locked_coin(send.build_psbt(None, None, FeeRate::from_sat_per_vb(10.0)).await);

    // so does a custom rate
    assert_rejects_locked_coin(
        send.get_custom_fee_option(
            Arc::new(FeeRate::from_sat_per_vb(7.0)),
            FeeSpeed::Custom { duration_mins: 10 },
        )
        .await,
    );

    // Next with the stale selection stops at an alert instead of the confirm screen
    assert_locked_coin_alert(alert_after_next(&send).await);

    // continuing from coin control again hands over only the unlocked coin, at every fee
    continue_from_coin_control(&coin_control, &send).await;
    assert_eq!(selected_outpoints(&send), HashSet::from([kept]));

    for fee_rate in [10.0, 5.0, 2.0, 7.0] {
        let psbt = send
            .build_psbt(None, None, FeeRate::from_sat_per_vb(fee_rate))
            .await
            .expect("the unlocked coin funds the send");
        assert_eq!(psbt_inputs(&psbt), HashSet::from([kept]), "at {fee_rate} sat/vB");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn coin_control_send_refuses_a_coin_locked_before_an_amount_edit() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    let Fixture { wallet, coin_control, send, to_lock, kept } =
        coin_control_send_of_two_coins().await;

    // lock one selected coin from its receive transaction's details
    let locked = wallet
        .manager
        .toggle_transaction_lock_state(Arc::new(TxId::from(to_lock.txid)))
        .await
        .expect("transaction lock toggles");
    assert_eq!(locked, crate::manager::wallet_manager::TransactionLockState::Locked);

    // edit the amount down to one the unlocked coin alone could cover
    let edited = 30_000;
    send.handle_coin_control_amount_changed(Amount::from_sat(edited));
    assert_eq!(send.state.lock().amount_sats, Some(edited));

    let medium = FeeRate::from_sat_per_vb(5.0);
    assert_rejects_locked_coin(send.build_psbt(None, None, medium).await);
    assert_locked_coin_alert(alert_after_next(&send).await);

    // the coin control screen reloads locks when it shows again, then Continue drops the coin
    coin_control.reload_labels().await.expect("coin control reloads locks");
    assert_eq!(coin_control_selection(&coin_control), HashSet::from([kept]));

    continue_from_coin_control(&coin_control, &send).await;
    send.handle_coin_control_amount_changed(Amount::from_sat(edited));

    let psbt = send.build_psbt(None, None, medium).await.expect("the unlocked coin funds the send");
    let recipient = external_recipient().script_pubkey();
    let paid = psbt
        .0
        .unsigned_tx
        .output
        .iter()
        .find(|output| output.script_pubkey == recipient)
        .expect("the send pays the recipient")
        .value;

    assert_eq!(psbt_inputs(&psbt), HashSet::from([kept]));
    assert_eq!(paid.to_sat(), edited);
}
