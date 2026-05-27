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
        if last_scan_finished.is_none() && matches!(scan_status, WalletScanStatus::Scanning(_)) {
            return Self::provisional();
        }

        Self::normal()
    }
}
