use super::{EnterMode, SendFlowAlertState};

pub(crate) fn amount_exceeds_spendable_balance(
    amount: Option<u64>,
    total_fee_sats: Option<u64>,
    spendable_balance: Option<u64>,
) -> bool {
    let amount = amount.unwrap_or(0);
    if amount == 0 {
        return false;
    }

    let Some(spendable) = spendable_balance else {
        return false;
    };

    total_spend_sats(amount, total_fee_sats) > spendable
}

pub(crate) fn total_spend_sats(amount: u64, total_fee_sats: Option<u64>) -> u64 {
    amount.saturating_add(total_fee_sats.unwrap_or(0))
}

pub(crate) fn spendable_balance_limit(
    unlocked_spendable_sats: Option<u64>,
    mode: &EnterMode,
) -> Option<u64> {
    match mode {
        EnterMode::SetAmount => unlocked_spendable_sats,
        EnterMode::CoinControl(coin_control) => Some(coin_control.max_send().as_sats()),
    }
}

pub(crate) fn spendable_balance_for_validation(spendable_balance: Option<u64>) -> u64 {
    // fail closed so amount validation cannot overspend locked or unknown UTXOs
    spendable_balance.unwrap_or(0)
}

pub(crate) fn unavailable_spendable_balance_alert(
    unlocked_spendable_sats: Option<u64>,
    lock_state_load_failed: bool,
    mode: &EnterMode,
) -> Option<SendFlowAlertState> {
    if mode.is_coin_control() {
        return None;
    }

    if lock_state_load_failed {
        return Some(SendFlowAlertState::General {
            title: "Unable to Read Locked Coins".to_string(),
            message: "Cove could not read the lock state for this wallet. Locked coins are excluded for safety. Please try again shortly.".to_string(),
        });
    }

    if unlocked_spendable_sats.is_none() {
        return Some(SendFlowAlertState::General {
            title: "Balance Still Loading".to_string(),
            message: "Cove is still checking which coins are unlocked. Please try again shortly."
                .to_string(),
        });
    }

    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn amount_exceeds_spendable_balance_uses_unlocked_balance() {
        assert!(super::amount_exceeds_spendable_balance(Some(6_000), None, Some(5_000)));
        assert!(!super::amount_exceeds_spendable_balance(Some(5_000), None, Some(5_000)));
        assert!(!super::amount_exceeds_spendable_balance(Some(0), None, Some(0)));
        assert!(!super::amount_exceeds_spendable_balance(Some(1), None, None));
        assert!(!super::amount_exceeds_spendable_balance(Some(0), None, None));
    }

    #[test]
    fn amount_exceeds_spendable_balance_includes_fee_when_available() {
        assert!(super::amount_exceeds_spendable_balance(Some(5_000), Some(156), Some(5_001)));
        assert!(!super::amount_exceeds_spendable_balance(Some(5_000), Some(1), Some(5_001)));
    }

    #[test]
    fn amount_exceeds_spendable_balance_falls_back_to_amount_without_fee() {
        assert!(!super::amount_exceeds_spendable_balance(Some(5_000), None, Some(5_001)));
    }

    #[test]
    fn zero_amount_does_not_exceed_spendable_balance_even_with_fee() {
        assert!(!super::amount_exceeds_spendable_balance(Some(0), Some(1), Some(0)));
    }

    #[test]
    fn validation_spendable_balance_uses_zero_when_unlocked_balance_is_unknown() {
        let spendable = super::spendable_balance_limit(Some(5_000), &super::EnterMode::SetAmount);
        assert_eq!(super::spendable_balance_for_validation(spendable), 5_000);

        let spendable = super::spendable_balance_limit(None, &super::EnterMode::SetAmount);
        assert_eq!(super::spendable_balance_for_validation(spendable), 0);
    }

    #[test]
    fn unavailable_balance_alert_distinguishes_lock_state_failures() {
        let alert =
            super::unavailable_spendable_balance_alert(None, true, &super::EnterMode::SetAmount);

        assert!(matches!(
            alert,
            Some(super::SendFlowAlertState::General { title, .. }) if title.contains("Locked")
        ));
    }

    #[test]
    fn unavailable_balance_alert_distinguishes_loading_state() {
        let alert =
            super::unavailable_spendable_balance_alert(None, false, &super::EnterMode::SetAmount);

        assert!(matches!(
            alert,
            Some(super::SendFlowAlertState::General { title, .. }) if title.contains("Loading")
        ));
        assert_eq!(
            super::unavailable_spendable_balance_alert(
                Some(5_000),
                false,
                &super::EnterMode::SetAmount
            ),
            None
        );
    }
}
