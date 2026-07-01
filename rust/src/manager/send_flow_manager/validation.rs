use super::SendFlowAlertState;

pub(crate) fn amount_exceeds_spendable_balance(
    amount: Option<u64>,
    spendable_balance: Option<u64>,
) -> bool {
    let amount = amount.unwrap_or(0);
    if amount == 0 {
        return false;
    }

    let Some(spendable) = spendable_balance else {
        return false;
    };

    amount > spendable
}

pub(crate) fn spendable_balance_for_validation(unlocked_spendable_sats: Option<u64>) -> u64 {
    // fail closed so amount validation cannot overspend locked or unknown UTXOs
    unlocked_spendable_sats.unwrap_or(0)
}

pub(crate) fn unavailable_spendable_balance_alert(
    unlocked_spendable_sats: Option<u64>,
    lock_state_load_failed: bool,
) -> Option<SendFlowAlertState> {
    if lock_state_load_failed {
        return Some(SendFlowAlertState::UnableToReadLockedCoins);
    }

    if unlocked_spendable_sats.is_none() {
        return Some(SendFlowAlertState::BalanceStillLoading);
    }

    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn amount_exceeds_spendable_balance_uses_unlocked_balance() {
        assert!(super::amount_exceeds_spendable_balance(Some(6_000), Some(5_000)));
        assert!(!super::amount_exceeds_spendable_balance(Some(5_000), Some(5_000)));
        assert!(!super::amount_exceeds_spendable_balance(Some(0), Some(0)));
        assert!(!super::amount_exceeds_spendable_balance(Some(1), None));
        assert!(!super::amount_exceeds_spendable_balance(Some(0), None));
    }

    #[test]
    fn validation_spendable_balance_uses_zero_when_unlocked_balance_is_unknown() {
        assert_eq!(super::spendable_balance_for_validation(Some(5_000)), 5_000);
        assert_eq!(super::spendable_balance_for_validation(None), 0);
    }

    #[test]
    fn unavailable_balance_alert_distinguishes_lock_state_failures() {
        let alert = super::unavailable_spendable_balance_alert(None, true);

        assert!(matches!(alert, Some(super::SendFlowAlertState::UnableToReadLockedCoins)));
    }

    #[test]
    fn unavailable_balance_alert_distinguishes_loading_state() {
        let alert = super::unavailable_spendable_balance_alert(None, false);

        assert!(matches!(alert, Some(super::SendFlowAlertState::BalanceStillLoading)));
        assert_eq!(super::unavailable_spendable_balance_alert(Some(5_000), false), None);
    }
}
