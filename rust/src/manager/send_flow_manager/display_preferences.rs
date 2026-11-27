use std::sync::Weak;

use cove_types::amount::Amount;
use tokio::sync::watch;
use tracing::debug;

use crate::{
    manager::wallet_manager::live_metadata::WalletDisplayPreferences, wallet::metadata::FiatOrBtc,
};

use super::{
    DeferredSender, Message, RustSendFlowManager,
    amount_input::{btc_entry_text, fiat_entry_text},
    state::{AmountEntryMode, SendFlowManagerState},
};

impl RustSendFlowManager {
    /// Applies the wallet's display preferences after the wallet manager changed them
    ///
    /// The unit or field and the entered text change in one critical section, the same one an
    /// amount edit is checked, parsed and committed in, and an edit typed in the old mode is
    /// dropped, so a keystroke is never parsed in a unit it was not typed in. Sensitive visibility
    /// is read live when amounts render, so any change only needs the presenters refreshed
    pub(crate) fn apply_display_preferences(&self, preferences: WalletDisplayPreferences) {
        let mut sender = self.reconciler.deferred_sender();
        apply_amount_entry_mode(&mut self.state.lock(), preferences.into(), &mut sender);
        sender.queue(Message::RefreshPresenters);
    }

    /// Follows the wallet's display preferences until the send flow or the wallet manager is gone
    pub(crate) async fn follow_display_preferences(
        manager: Weak<Self>,
        mut preferences: watch::Receiver<WalletDisplayPreferences>,
    ) {
        while preferences.changed().await.is_ok() {
            let current = *preferences.borrow_and_update();
            let Some(manager) = manager.upgrade() else { return };
            manager.apply_display_preferences(current);
        }

        debug!("wallet display preferences closed, send flow stops following them");
    }
}

/// Moves the amount entry to a new mode, rewriting the entered text for the new unit or field
fn apply_amount_entry_mode(
    state: &mut SendFlowManagerState,
    mode: AmountEntryMode,
    sender: &mut DeferredSender,
) {
    let old = state.amount_entry.mode;
    if old == mode {
        return;
    }

    state.amount_entry.mode = mode;
    sender.queue(Message::UpdateAmountEntryMode(mode));

    let amount = state.amount_sats.map(Amount::from_sat);
    let unit_changed = old.unit != mode.unit;
    let switched_to_btc = old.fiat_or_btc != mode.fiat_or_btc && mode.fiat_or_btc == FiatOrBtc::Btc;

    // the bitcoin text is rewritten in the new unit, and partial input with no amount yet, such
    // as "0." in BTC, has no meaning in sats, so it is cleared
    if unit_changed || switched_to_btc {
        let text = amount.map(|amount| btc_entry_text(amount, mode.unit)).unwrap_or_default();
        set_text(&mut state.amount_entry.btc_text, text, sender, Message::UpdateEnteringBtcAmount);
    }

    let switched_to_fiat =
        old.fiat_or_btc != mode.fiat_or_btc && mode.fiat_or_btc == FiatOrBtc::Fiat;

    if switched_to_fiat && amount.is_some() {
        let amount_fiat = state.amount_fiat.unwrap_or_default();
        let text = fiat_entry_text(amount_fiat, state.selected_fiat_currency);
        set_text(
            &mut state.amount_entry.fiat_text,
            text,
            sender,
            Message::UpdateEnteringFiatAmount,
        );
    }
}

fn set_text(
    current: &mut String,
    text: String,
    sender: &mut DeferredSender,
    message: fn(String) -> Message,
) {
    if *current != text {
        current.clone_from(&text);
        sender.queue(message(text));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use act_zero::{call, send};
    use cove_types::unit::BitcoinUnit;

    use crate::{
        manager::{
            deferred_sender::SingleOrMany,
            send_flow_manager::{RustSendFlowManager, SendFlowManagerAction},
            wallet_manager::{RustWalletManager, WalletManagerAction},
        },
        wallet::{
            balance::Balance,
            metadata::{FiatOrBtc, WalletMetadata},
        },
    };

    use super::Message;

    /// A live send flow over a wallet showing BTC, typing in bitcoin, with amounts visible
    fn send_flow_over_btc_wallet() -> (Arc<RustWalletManager>, Arc<RustSendFlowManager>) {
        crate::test_support::ensure_tokio_runtime();
        crate::database::test_support::init_test_database();

        let mut metadata = WalletMetadata::preview_new();
        metadata.selected_unit = BitcoinUnit::Btc;
        metadata.fiat_or_btc = FiatOrBtc::Btc;
        metadata.sensitive_visible = true;

        let wallet = Arc::new(RustWalletManager::preview_new_wallet_with_metadata(metadata));
        let send = RustSendFlowManager::with_wallet(wallet.clone(), Arc::new(Balance::default()));
        (wallet, send)
    }

    async fn wallet_dispatch(wallet: &RustWalletManager, action: WalletManagerAction) {
        call!(wallet.actor.dispatch_metadata_action(action)).await.expect("wallet actor responds");
    }

    fn type_btc_amount(send: &Arc<RustSendFlowManager>, text: &str) {
        type_btc_amount_in(send, text, send.amount_entry_mode().unit);
    }

    /// An edit the platform sends for text typed while the field showed `unit`
    fn type_btc_amount_in(send: &Arc<RustSendFlowManager>, text: &str, unit: BitcoinUnit) {
        let action =
            SendFlowManagerAction::NotifyEnteringBtcAmountChanged { text: text.to_string(), unit };
        send.clone().dispatch(action);
    }

    fn discard_messages(send: &RustSendFlowManager) {
        while send.reconciler.receiver().try_recv().is_ok() {}
    }

    /// Waits for the send flow to reconcile a change the wallet manager published
    async fn messages_until_refresh(send: &RustSendFlowManager) -> Vec<Message> {
        let receiver = send.reconciler.receiver();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut messages = Vec::new();

        while !messages.contains(&Message::RefreshPresenters) {
            let next = tokio::time::timeout_at(deadline, receiver.recv_async())
                .await
                .expect("the send flow reconciles the change in time")
                .expect("reconcile channel is open");

            match next {
                SingleOrMany::Single(message) => messages.push(message),
                SingleOrMany::Many(batch) => messages.extend(batch),
            }
        }

        messages
    }

    #[tokio::test(flavor = "current_thread")]
    async fn wallet_display_changes_reach_a_live_send_flow_with_the_amount_converted() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;
        let (wallet, send) = send_flow_over_btc_wallet();

        type_btc_amount(&send, "0.0015");
        assert_eq!(send.amount_sats(), 150_000);
        discard_messages(&send);

        // sats picked on another screen: the text and unit flip together and the amount stays
        wallet_dispatch(&wallet, WalletManagerAction::UpdateUnit(BitcoinUnit::Sat)).await;
        let messages = messages_until_refresh(&send).await;

        assert!(messages.contains(&Message::UpdateEnteringBtcAmount("150,000".to_string())));
        assert!(messages.iter().any(|message| matches!(
            message,
            Message::UpdateAmountEntryMode(mode) if mode.unit == BitcoinUnit::Sat
        )));
        assert_eq!(send.amount_entry_mode().unit, BitcoinUnit::Sat);
        assert_eq!(send.amount_sats(), 150_000);
        assert_eq!(send.send_amount_btc(), "150,000");

        // the next keystroke parses in sats, not BTC
        type_btc_amount(&send, "150,0001");
        assert_eq!(send.amount_sats(), 1_500_001);
        discard_messages(&send);

        // typing in fiat rewrites the fiat field from the amount
        send.state.lock().amount_fiat = Some(1_500.0);
        wallet_dispatch(&wallet, WalletManagerAction::ToggleFiatOrBtc).await;
        let messages = messages_until_refresh(&send).await;

        assert_eq!(send.amount_entry_mode().fiat_or_btc, FiatOrBtc::Fiat);
        assert!(messages.iter().any(|message| matches!(
            message,
            Message::UpdateEnteringFiatAmount(text) if text.contains("1,500")
        )));

        // hiding sensitive amounts masks fiat at once and leaves the entry mode alone
        assert_ne!(send.display_fiat_amount(10.0, true), "**************");
        wallet_dispatch(&wallet, WalletManagerAction::ToggleSensitiveVisibility).await;
        let messages = messages_until_refresh(&send).await;

        assert!(
            !messages.iter().any(|message| matches!(message, Message::UpdateAmountEntryMode(_)))
        );
        assert_eq!(send.display_fiat_amount(10.0, true), "**************");
    }

    /// Android sends a keystroke after a short delay, so an edit typed in sats can reach Rust
    /// after the wallet moved to BTC; parsed in BTC it would make 500,001 sats 500,001 BTC
    #[tokio::test(flavor = "current_thread")]
    async fn an_edit_typed_before_a_unit_change_is_dropped_not_parsed_in_the_new_unit() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;
        let (wallet, send) = send_flow_over_btc_wallet();

        wallet_dispatch(&wallet, WalletManagerAction::UpdateUnit(BitcoinUnit::Sat)).await;
        messages_until_refresh(&send).await;
        type_btc_amount(&send, "50000");
        assert_eq!(send.amount_sats(), 50_000);

        wallet_dispatch(&wallet, WalletManagerAction::UpdateUnit(BitcoinUnit::Btc)).await;
        messages_until_refresh(&send).await;
        assert_eq!(send.state.lock().amount_entry.btc_text, "0.0005");

        // the sats keystroke still on its way lands after the switch
        type_btc_amount_in(&send, "500001", BitcoinUnit::Sat);
        assert_eq!(send.amount_sats(), 50_000);
        assert_eq!(send.state.lock().amount_entry.btc_text, "0.0005");

        // and the field is told what Rust holds, so the platform drops the stale text too
        let resent = std::iter::from_fn(|| send.reconciler.receiver().try_recv().ok())
            .flat_map(|message| match message {
                SingleOrMany::Single(message) => vec![message],
                SingleOrMany::Many(batch) => batch,
            })
            .collect::<Vec<_>>();
        assert!(resent.contains(&Message::UpdateEnteringBtcAmount("0.0005".to_string())));

        // an edit typed in BTC is parsed as one
        type_btc_amount(&send, "0.00051");
        assert_eq!(send.amount_sats(), 51_000);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_send_flow_starts_in_the_unit_the_wallet_shows_when_it_is_created() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;
        let (wallet, btc_send) = send_flow_over_btc_wallet();
        drop(btc_send);

        wallet_dispatch(&wallet, WalletManagerAction::UpdateUnit(BitcoinUnit::Sat)).await;
        let send = RustSendFlowManager::with_wallet(wallet, Arc::new(Balance::default()));
        assert_eq!(send.amount_entry_mode().unit, BitcoinUnit::Sat);

        type_btc_amount(&send, "50000");
        assert_eq!(send.amount_sats(), 50_000);
    }

    /// A send flow created while a unit change is still on its way through the wallet actor
    /// starts on the old unit, and the change landing afterwards moves it to the new one with the
    /// typed amount converted
    #[tokio::test(flavor = "current_thread")]
    async fn a_unit_change_landing_after_the_send_flow_is_created_converts_the_amount() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;
        let (wallet, send) = send_flow_over_btc_wallet();
        type_btc_amount(&send, "0.0005");
        assert_eq!(send.amount_sats(), 50_000);

        // queued on the actor, not awaited, as a change from the wallet screen would be
        send!(
            wallet
                .actor
                .dispatch_metadata_action(WalletManagerAction::UpdateUnit(BitcoinUnit::Sat))
        );

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        while send.amount_entry_mode().unit != BitcoinUnit::Sat {
            assert!(tokio::time::Instant::now() < deadline, "the queued unit change never arrived");
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        assert_eq!(send.state.lock().amount_entry.btc_text, "50,000");
        assert_eq!(send.amount_sats(), 50_000);

        // and the next keystroke parses in sats
        type_btc_amount(&send, "50,0001");
        assert_eq!(send.amount_sats(), 500_001);
    }
}
