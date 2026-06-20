use std::sync::Arc;

use cove_tap_card::TapSigner;
use cove_types::{Network, TxId, WalletId, address::Address, amount::Amount};

use crate::router::AfterPinAction;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum AlertDisplayType {
    FullAlert,
    Toast,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum AppAlertState {
    // success
    ImportedSuccessfully,
    ImportedLabelsSuccessfully,

    // warn
    DuplicateWallet { wallet_id: WalletId },
    HotWalletKeyMissing { wallet_id: WalletId },

    // errors
    InvalidWordGroup,
    ErrorImportingHotWallet { message: String },
    AddressWrongNetwork { address: Arc<Address>, network: Network, current_network: Network },
    FoundAddress { address: Arc<Address>, amount: Option<Arc<Amount>> },
    UnableToSelectWallet,
    ErrorImportingHardwareWallet { message: String },
    InvalidFileFormat { message: String },
    NoWalletSelected { address: Arc<Address> },
    InvalidFormat { message: String },
    NoUnsignedTransactionFound { tx_id: Arc<TxId> },
    UnableToGetAddress { error: String },
    NoCameraPermission,
    FailedToScanQr { error: String },
    CantSendOnWatchOnlyWallet,
    TapSignerSetupFailed { message: String },
    TapSignerDeriveFailed { message: String },
    TapSignerInvalidAuth,
    TapSignerNoBackup { tap_signer: Arc<TapSigner> },
    TapSignerWrongPin { tap_signer: Arc<TapSigner>, action: AfterPinAction },

    // database corruption
    WalletDatabaseCorrupted { wallet_id: WalletId, error: String },

    // generic message or error
    General { title: String, message: String },

    // loading popup with progress indicator
    Loading,

    // confirmation
    ConfirmWatchOnly,

    // watch-only import sub-alerts
    WatchOnlyImportHardware,
    WatchOnlyImportWords,

    // action
    UninitializedTapSigner { tap_signer: Arc<TapSigner> },
    TapSignerWalletFound { wallet_id: WalletId },
    InitializedTapSigner { tap_signer: Arc<TapSigner> },
}

#[uniffi::export]
impl AppAlertState {
    pub fn is_equal(&self, rhs: &Self) -> bool {
        self == rhs
    }

    pub fn display_type(&self) -> AlertDisplayType {
        match self {
            Self::ImportedLabelsSuccessfully
            | Self::UnableToGetAddress { .. }
            | Self::FailedToScanQr { .. } => AlertDisplayType::Toast,
            _ => AlertDisplayType::FullAlert,
        }
    }
}
