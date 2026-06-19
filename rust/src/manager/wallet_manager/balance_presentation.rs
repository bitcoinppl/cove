use super::ledger_state::{InitialScanActivity, WalletLedgerState};

#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct BalancePresentation {
    pub primary_opacity: f64,
    pub secondary_opacity: f64,
    pub pending_opacity: f64,
}

impl BalancePresentation {
    const fn normal() -> Self {
        Self { primary_opacity: 1.0, secondary_opacity: 0.75, pending_opacity: 0.6 }
    }

    const fn provisional() -> Self {
        Self { primary_opacity: 0.48, secondary_opacity: 0.42, pending_opacity: 0.38 }
    }

    pub(crate) fn for_ledger_state(ledger_state: WalletLedgerState) -> Self {
        match ledger_state {
            WalletLedgerState::Complete => Self::normal(),
            WalletLedgerState::InitialScanIncomplete(InitialScanActivity::Active) => {
                Self::provisional()
            }
            WalletLedgerState::InitialScanIncomplete(InitialScanActivity::Idle) => {
                Self::provisional()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_ledger_uses_normal_balance_presentation() {
        assert_eq!(
            BalancePresentation::for_ledger_state(WalletLedgerState::Complete),
            BalancePresentation::normal()
        );
    }

    #[test]
    fn incomplete_active_initial_scan_uses_provisional_balance_presentation() {
        assert_eq!(
            BalancePresentation::for_ledger_state(WalletLedgerState::InitialScanIncomplete(
                InitialScanActivity::Active
            )),
            BalancePresentation::provisional()
        );
    }

    #[test]
    fn incomplete_idle_initial_scan_uses_provisional_balance_presentation() {
        assert_eq!(
            BalancePresentation::for_ledger_state(WalletLedgerState::InitialScanIncomplete(
                InitialScanActivity::Idle
            )),
            BalancePresentation::provisional()
        );
    }
}
