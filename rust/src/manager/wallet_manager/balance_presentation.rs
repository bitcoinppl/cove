use std::time::Duration;

use super::WalletScanStatus;

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

    pub(crate) fn for_scan(
        last_scan_finished: Option<Duration>,
        scan_status: &WalletScanStatus,
    ) -> Self {
        let scan_is_active = matches!(
            scan_status,
            WalletScanStatus::Scanning(_) | WalletScanStatus::ScanningPendingProgress(_)
        );
        if last_scan_finished.is_none() && scan_is_active {
            return Self::provisional();
        }

        Self::normal()
    }
}
