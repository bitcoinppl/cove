use crate::wallet::metadata::WalletMetadata;

use super::WalletScanStatus;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum InitialScanCompletion {
    Complete,
    Incomplete,
}

impl From<&WalletMetadata> for InitialScanCompletion {
    fn from(metadata: &WalletMetadata) -> Self {
        if metadata.internal.performed_full_scan_at.is_some() {
            return Self::Complete;
        }

        Self::Incomplete
    }
}

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum InitialScanActivity {
    Active,
    Idle,
}

impl InitialScanActivity {
    fn from_scan_status(scan_status: &WalletScanStatus) -> Self {
        match scan_status {
            WalletScanStatus::Scanning(_) | WalletScanStatus::ScanningPendingProgress(_) => {
                Self::Active
            }
            WalletScanStatus::Idle => Self::Idle,
        }
    }
}

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum WalletLedgerState {
    Complete,
    InitialScanIncomplete(InitialScanActivity),
}

impl WalletLedgerState {
    pub(crate) fn from_metadata_and_scan_status(
        metadata: &WalletMetadata,
        scan_status: &WalletScanStatus,
    ) -> Self {
        Self::from_parts(metadata.into(), scan_status)
    }

    fn from_parts(completion: InitialScanCompletion, scan_status: &WalletScanStatus) -> Self {
        match completion {
            InitialScanCompletion::Complete => Self::Complete,
            InitialScanCompletion::Incomplete => {
                Self::InitialScanIncomplete(InitialScanActivity::from_scan_status(scan_status))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    use crate::manager::wallet_manager::{WalletScanPhase, WalletScanProgress};

    fn progress() -> WalletScanProgress {
        WalletScanProgress {
            phase: WalletScanPhase::Full,
            checked: 0,
            gap: 0,
            stop_gap: 150,
            progress_basis_points: 0,
        }
    }

    #[test]
    fn complete_initial_scan_produces_complete_ledger_state() {
        let state = WalletLedgerState::from_parts(
            InitialScanCompletion::Complete,
            &WalletScanStatus::Scanning(progress()),
        );

        assert_eq!(state, WalletLedgerState::Complete);
    }

    #[test]
    fn incomplete_initial_scan_preserves_active_scan_activity() {
        let state = WalletLedgerState::from_parts(
            InitialScanCompletion::Incomplete,
            &WalletScanStatus::ScanningPendingProgress(WalletScanPhase::Full),
        );

        assert_eq!(state, WalletLedgerState::InitialScanIncomplete(InitialScanActivity::Active));
    }

    #[test]
    fn incomplete_initial_scan_preserves_idle_scan_activity() {
        let state = WalletLedgerState::from_parts(
            InitialScanCompletion::Incomplete,
            &WalletScanStatus::Idle,
        );

        assert_eq!(state, WalletLedgerState::InitialScanIncomplete(InitialScanActivity::Idle));
    }

    #[test]
    fn last_scan_finished_does_not_complete_initial_scan() {
        let mut metadata = WalletMetadata::preview_new();
        metadata.internal.last_scan_finished = Some(Duration::from_secs(1));
        metadata.internal.performed_full_scan_at = None;

        let state =
            WalletLedgerState::from_metadata_and_scan_status(&metadata, &WalletScanStatus::Idle);

        assert_eq!(state, WalletLedgerState::InitialScanIncomplete(InitialScanActivity::Idle));
    }
}
