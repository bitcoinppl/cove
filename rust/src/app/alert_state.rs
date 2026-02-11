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

    // generic message or error
    General { title: String, message: String },

    // loading popup with progress indicator
    Loading,

    // confirmation
    ConfirmWatchOnly,

    // action
    UninitializedTapSigner { tap_signer: Arc<TapSigner> },
    TapSignerWalletFound { wallet_id: WalletId },
    InitializedTapSigner { tap_signer: Arc<TapSigner> },
}

#[uniffi::export]
impl AppAlertState {
    pub fn title(&self) -> String {
        match self {
            Self::InvalidWordGroup => "Words Not Valid",
            Self::DuplicateWallet { .. } => "Duplicate Wallet",
            Self::HotWalletKeyMissing { .. } => "Wallet Needs Recovery",
            Self::ErrorImportingHotWallet { .. } => "Error",
            Self::ImportedSuccessfully | Self::ImportedLabelsSuccessfully => "Success",
            Self::UnableToSelectWallet => "Error",
            Self::ErrorImportingHardwareWallet { .. } => "Error Importing Hardware Wallet",
            Self::InvalidFileFormat { .. } => "Invalid File Format",
            Self::InvalidFormat { .. } => "Invalid Format",
            Self::AddressWrongNetwork { .. } => "Wrong Network",
            Self::NoWalletSelected { .. } => "Select a Wallet",
            Self::FoundAddress { .. } => "Found Address",
            Self::NoCameraPermission => "Camera Access is Required",
            Self::FailedToScanQr { .. } => "Failed to Scan QR",
            Self::NoUnsignedTransactionFound { .. } => "No Unsigned Transaction Found",
            Self::UnableToGetAddress { .. } => "Unable to Get Address",
            Self::CantSendOnWatchOnlyWallet | Self::ConfirmWatchOnly => "Watch Only Wallet",
            Self::UninitializedTapSigner { .. } => "Setup TAPSIGNER?",
            Self::TapSignerSetupFailed { .. } => "Setup Failed",
            Self::TapSignerDeriveFailed { .. } => "TAPSIGNER Import Failed",
            Self::TapSignerInvalidAuth | Self::TapSignerWrongPin { .. } => "Wrong PIN",
            Self::TapSignerWalletFound { .. } => "Wallet Found",
            Self::InitializedTapSigner { .. } => "Import TAPSIGNER?",
            Self::TapSignerNoBackup { .. } => "No Backup Found",
            Self::General { title, .. } => title,
            Self::Loading => "Working on it...",
        }
        .to_string()
    }

    pub fn message(&self) -> String {
        match self {
            Self::InvalidWordGroup => {
                "The words do not create a valid wallet. Please check the words and try again."
                    .to_string()
            }
            Self::DuplicateWallet { .. } => {
                "This wallet has already been imported! Taking you there now...".to_string()
            }
            Self::HotWalletKeyMissing { .. } => {
                let base = "This wallet's private key is no longer available on this device. It has been converted to watch-only. To restore full access, import your seed words.";

                #[cfg(target_os = "ios")]
                let backup_type = "iCloud";

                #[cfg(target_os = "android")]
                let backup_type = "Android";

                #[cfg(not(any(target_os = "ios", target_os = "android")))]
                let backup_type = "device";

                format!("{base}\n\nThis can happen when restoring from a backup to a new phone. For security reasons, private keys are not included in regular {backup_type} backups.")
            }
            Self::ConfirmWatchOnly => {
                "You will not be able to send any bitcoin with this wallet. You will only be able to create receive addresses and view transactions."
                    .to_string()
            }
            Self::ErrorImportingHotWallet { message } => message.clone(),
            Self::ImportedSuccessfully => "Wallet imported successfully".to_string(),
            Self::ImportedLabelsSuccessfully => "Labels imported successfully".to_string(),
            Self::UnableToSelectWallet => {
                "Unable to select wallet, please try again".to_string()
            }
            Self::ErrorImportingHardwareWallet { message } => {
                format!("Error importing hardware wallet, more info: {message}")
            }
            Self::InvalidFileFormat { message } => message.clone(),
            Self::InvalidFormat { message } => message.clone(),
            Self::AddressWrongNetwork {
                address,
                network,
                current_network,
            } => {
                format!(
                    "The address {} is on the wrong network. You are on {}, and the address was for {}.",
                    address,
                    current_network,
                    network
                )
            }
            Self::NoWalletSelected { .. } => {
                "Please select a wallet to send to this address".to_string()
            }
            Self::FoundAddress { address, .. } => format!("Address: {}", address.spaced_out()),
            Self::NoCameraPermission => {
                "Please allow camera access in Settings to use this feature.".to_string()
            }
            Self::FailedToScanQr { error } => {
                format!("Error scanning QR code, more info: {error}")
            }
            Self::NoUnsignedTransactionFound { tx_id } => {
                format!(
                    "No unsigned transaction found for transaction {}",
                    tx_id.as_hash_string()
                )
            }
            Self::UnableToGetAddress { error } => {
                format!("Error getting address, more info: {error}")
            }
            Self::CantSendOnWatchOnlyWallet => {
                "You cannot send from a watch-only wallet".to_string()
            }
            Self::UninitializedTapSigner { .. } => {
                "This TAPSIGNER has not been set up yet. Would you like to set it up now?"
                    .to_string()
            }
            Self::TapSignerSetupFailed { message } => {
                format!("Error setting up TAPSIGNER, more info: {message}")
            }
            Self::TapSignerDeriveFailed { message } => {
                format!("Error importing TAPSIGNER, more info: {message}")
            }
            Self::TapSignerInvalidAuth | Self::TapSignerWrongPin { .. } => {
                "The PIN you entered was incorrect. Please try again.".to_string()
            }
            Self::TapSignerWalletFound { .. } => {
                "Would you like to go to this wallet?".to_string()
            }
            Self::InitializedTapSigner { .. } => {
                "Would you like to start using this TAPSIGNER with Cove?".to_string()
            }
            Self::TapSignerNoBackup { .. } => {
                "Can't change the PIN without taking a backup of the wallet. Would you like to take a backup now?"
                    .to_string()
            }
            Self::General { message, .. } => message.clone(),
            Self::Loading => String::new(),
        }
    }

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
