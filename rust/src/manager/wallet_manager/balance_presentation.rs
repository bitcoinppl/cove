use super::ledger_state::WalletLedgerState;

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
            WalletLedgerState::InitialScanIncomplete(_) => Self::provisional(),
        }
    }
}

/// Returns provisional presentation values for loading screens before a wallet manager is available
#[uniffi::export]
pub fn balance_presentation_provisional() -> BalancePresentation {
    BalancePresentation::provisional()
}
