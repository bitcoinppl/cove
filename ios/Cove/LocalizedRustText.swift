import SwiftUI

extension Network {
    var localizedDisplayName: String {
        switch self {
        case .bitcoin:
            String(localized: "Bitcoin")
        case .testnet:
            String(localized: "Testnet")
        case .testnet4:
            String(localized: "Testnet 4")
        case .signet:
            String(localized: "Signet")
        }
    }
}

extension WalletType {
    var localizedDisplayName: String {
        switch self {
        case .hot:
            String(localized: "On This Device")
        case .cold:
            String(localized: "Hardware Wallet")
        case .xpubOnly:
            String(localized: "Xpub Only")
        case .watchOnly:
            String(localized: "Watch Only")
        }
    }
}

extension WalletSecretType {
    var localizedDisplayName: String {
        switch self {
        case .mnemonic:
            String(localized: "Recovery Words")
        case .tapSignerBackup:
            String(localized: "TAPSIGNER")
        case .none:
            String(localized: "Xpub Only")
        case .unknown:
            String(localized: "Unknown")
        }
    }
}

extension WalletAddressType {
    var localizedDisplayName: String {
        switch self {
        case .nativeSegwit:
            String(localized: "Native Segwit")
        case .wrappedSegwit:
            String(localized: "Wrapped Segwit")
        case .legacy:
            String(localized: "Legacy")
        }
    }
}

extension CloudCheckIssue {
    var localizedMessage: String {
        switch self {
        case .offline:
            String(localized: "You're offline, so Cove can't check for a cloud backup right now. You can continue onboarding now and check Cloud Backup later in Settings.")
        case .cloudUnavailable:
            String(localized: "Cove couldn't confirm whether a cloud backup is available because cloud storage may be unavailable. You can still try restoring with your passkey if you're reinstalling this device.")
        case .unknown:
            String(localized: "Cove couldn't confirm whether a cloud backup is available. You can still try restoring with your passkey if you're reinstalling this device.")
        }
    }
}

extension AfterPinAction {
    var localizedUserMessage: String {
        switch self {
        case .derive:
            String(localized: "For security purposes, enter your TAPSIGNER PIN before importing your wallet.")
        case .change:
            String(localized: "Please enter your current PIN.")
        case .backup:
            String(localized: "For security purposes, enter your TAPSIGNER PIN before backing up your wallet.")
        case .sign:
            String(localized: "For security purposes, enter your TAPSIGNER PIN before signing a transaction.")
        }
    }
}

extension ScanProgress {
    var localizedDisplayText: String {
        switch self {
        case let .bbqr(scanned, total):
            String(localized: "Scanned \(scanned) of \(total)")
        case let .ur(percentage):
            String(localized: "Scanned \(UInt32(percentage * 100.0))%")
        }
    }

    var localizedDetailText: String? {
        switch self {
        case let .bbqr(scanned, total):
            let remaining = total.saturatingSubtract(scanned)
            if remaining == 1 {
                return String(localized: "1 part left")
            }

            return String(localized: "\(remaining) parts left")
        case .ur:
            return nil
        }
    }
}

private extension UInt32 {
    func saturatingSubtract(_ value: UInt32) -> UInt32 {
        self > value ? self - value : 0
    }
}

extension WalletMetadata {
    var localizedDeletionWarningMessage: String {
        if walletType == .hot, !verified {
            return String(localized: "This wallet is not backed up. Make sure you have your recovery words saved before deleting.")
        }

        return String(localized: "This action cannot be undone.")
    }
}

extension DeepVerificationFailure {
    var localizedMessage: String {
        switch self {
        case .retry:
            String(localized: "Cloud Backup verification failed. Try again.")
        case .recreateManifest:
            String(localized: "Cloud Backup needs to repair its manifest.")
        case .reinitializeBackup:
            String(localized: "Cloud Backup needs to be set up again.")
        case .unsupportedVersion:
            String(localized: "This Cloud Backup uses a newer format. Update Cove before continuing.")
        }
    }

    var localizedWarning: String? {
        switch self {
        case .retry, .unsupportedVersion:
            nil
        case .recreateManifest:
            String(localized: "Cove will rebuild the cloud backup manifest from the wallets on this device.")
        case .reinitializeBackup:
            String(localized: "Existing cloud backup data cannot be trusted. Set up Cloud Backup again to continue.")
        }
    }
}

extension CloudBackupFailure {
    var localizedMessage: String {
        String(localized: "Cloud Backup failed. Please try again.")
    }
}

extension BackupError {
    var localizedMessage: String {
        switch self {
        case .PasswordTooShort:
            String(localized: "Password must be at least 20 characters.")
        case .DecryptionFailed:
            String(localized: "Wrong password or corrupted backup file.")
        case .InvalidFormat:
            String(localized: "This backup file is not valid.")
        case .FileTooLarge:
            String(localized: "This backup file is too large.")
        case .UnsupportedVersion, .UnsupportedPayloadVersion:
            String(localized: "This backup was created by a newer version of Cove. Please update Cove and try again.")
        case .Truncated:
            String(localized: "This backup file is incomplete.")
        case .Encryption, .Serialization, .Deserialization, .Gather, .Restore, .Keychain, .Database, .Decompression:
            String(localized: "Unable to process this backup. Please try again.")
        }
    }
}

extension SendFlowError {
    var localizedTitle: String {
        switch self {
        case .EmptyAddress, .InvalidAddress, .WrongNetwork:
            String(localized: "Invalid Address")
        case .InvalidNumber, .ZeroAmount:
            String(localized: "Invalid Amount")
        case .InsufficientFunds, .NoBalance:
            String(localized: "Insufficient Funds")
        case .SendAmountToLow:
            String(localized: "Send Amount Too Low")
        case .UnableToGetFeeRate:
            String(localized: "Unable to Get Fee Rate")
        case .UnableToBuildTxn:
            String(localized: "Unable to Build Transaction")
        case .UnableToGetMaxSend:
            String(localized: "Unable to Get Max Send")
        case .UnableToSaveUnsignedTransaction:
            String(localized: "Unable to Save Unsigned Transaction")
        case .WalletManager(.LockedOutputsSelected):
            String(localized: "Insufficient Funds")
        case .WalletManager:
            String(localized: "Error")
        case .UnableToGetFeeDetails:
            String(localized: "Fee Details Error")
        }
    }

    var localizedMessage: String {
        switch self {
        case .EmptyAddress:
            String(localized: "Please enter an address.")
        case .InvalidNumber:
            String(localized: "Please enter a valid amount to send.")
        case .ZeroAmount:
            String(localized: "Enter an amount greater than zero.")
        case .NoBalance:
            String(localized: "You do not have any bitcoin in this wallet. Add bitcoin before sending a transaction.")
        case let .InvalidAddress(address):
            String(localized: "The address \(address) is invalid.")
        case let .WrongNetwork(address, validFor, current):
            String(localized: "The address \(address) is for \(validFor.localizedDisplayName), but your wallet is on \(current.localizedDisplayName).")
        case .InsufficientFunds:
            String(localized: "You do not have enough bitcoin to cover the amount plus fees.")
        case .SendAmountToLow:
            String(localized: "Send amount is too low. Please send at least 5000 sats.")
        case .UnableToGetFeeRate:
            String(localized: "Check your internet connection and try again.")
        case .WalletManager(.LockedOutputsSelected):
            String(localized: "Selected coins include locked coins. Unlock them or choose different coins.")
        case .WalletManager:
            String(localized: "Unable to complete the wallet operation. Please try again.")
        case .UnableToGetFeeDetails:
            String(localized: "Unable to get fee details. Please try again.")
        case .UnableToBuildTxn:
            String(localized: "Unable to build the transaction. Please try again.")
        case .UnableToGetMaxSend:
            String(localized: "Unable to calculate the maximum send amount. Please try again.")
        case .UnableToSaveUnsignedTransaction:
            String(localized: "Unable to save the unsigned transaction. Please try again.")
        }
    }
}

extension SendFlowAlertState {
    var localizedTitle: String {
        switch self {
        case let .error(error):
            error.localizedTitle
        case let .general(title, _):
            title
        case .unableToLoadFees:
            String(localized: "Unable to Load Fees")
        case .feeTooHigh:
            String(localized: "Fee Too High")
        case .highFeeWarning:
            String(localized: "High Fee Warning")
        case .unableToReadLockedCoins:
            String(localized: "Unable to Read Locked Coins")
        case .balanceStillLoading:
            String(localized: "Balance Still Loading")
        }
    }

    var localizedMessage: String {
        switch self {
        case let .error(error):
            error.localizedMessage
        case let .general(_, message):
            message
        case .unableToLoadFees:
            String(localized: "Cannot create a transaction without fee information. Check your internet connection and try again.")
        case .feeTooHigh:
            String(localized: "The fee is higher than the amount you are sending.")
        case .highFeeWarning:
            String(localized: "The fee is higher than 20% of the amount you are sending.")
        case .unableToReadLockedCoins:
            String(localized: "Cove could not read locked coin selections. Try again before sending.")
        case .balanceStillLoading:
            String(localized: "Your wallet balance is still loading. Try again in a moment.")
        }
    }
}

extension AppAlertState {
    var localizedTitle: String {
        switch self {
        case .invalidWordGroup:
            String(localized: "Words Not Valid")
        case .duplicateWallet:
            String(localized: "Duplicate Wallet")
        case .hotWalletKeyMissing:
            String(localized: "Wallet Needs Recovery")
        case .errorImportingHotWallet:
            String(localized: "Error")
        case .importedSuccessfully, .importedLabelsSuccessfully:
            String(localized: "Success")
        case .unableToSelectWallet:
            String(localized: "Error")
        case .errorImportingHardwareWallet:
            String(localized: "Error Importing Hardware Wallet")
        case .invalidFileFormat:
            String(localized: "Invalid File Format")
        case .invalidFormat:
            String(localized: "Invalid Format")
        case .addressWrongNetwork:
            String(localized: "Wrong Network")
        case .noWalletSelected:
            String(localized: "Select a Wallet")
        case .foundAddress:
            String(localized: "Found Address")
        case .noCameraPermission:
            String(localized: "Camera Access is Required")
        case .failedToScanQr:
            String(localized: "Failed to Scan QR")
        case .noUnsignedTransactionFound:
            String(localized: "No Unsigned Transaction Found")
        case .unableToGetAddress:
            String(localized: "Unable to Get Address")
        case .cantSendOnWatchOnlyWallet, .confirmWatchOnly:
            String(localized: "Watch Only Wallet")
        case .watchOnlyImportHardware:
            String(localized: "Import Hardware Wallet")
        case .watchOnlyImportWords:
            String(localized: "Import Words")
        case .uninitializedTapSigner:
            String(localized: "Set Up TAPSIGNER?")
        case .tapSignerSetupFailed:
            String(localized: "Setup Failed")
        case .tapSignerDeriveFailed:
            String(localized: "TAPSIGNER Import Failed")
        case .tapSignerInvalidAuth, .tapSignerWrongPin:
            String(localized: "Wrong PIN")
        case .tapSignerWalletFound:
            String(localized: "Wallet Found")
        case .initializedTapSigner:
            String(localized: "Import TAPSIGNER?")
        case .tapSignerNoBackup:
            String(localized: "No Backup Found")
        case .walletDatabaseCorrupted:
            String(localized: "Wallet Database Error")
        case let .general(title, _):
            title
        case .loading:
            String(localized: "Working on it...")
        }
    }

    var localizedMessage: String {
        switch self {
        case .invalidWordGroup:
            String(localized: "The words do not create a valid wallet. Please check the words and try again.")
        case .duplicateWallet:
            String(localized: "This wallet has already been imported. Taking you there now...")
        case .hotWalletKeyMissing:
            String(localized: "This wallet's private key is no longer available on this device. It has been converted to watch-only. To restore full access, recover it from Cloud Backup or import your seed words.\n\nThis can happen when restoring from a backup to a new phone. For security reasons, private keys are not included in regular iOS device backups.")
        case .confirmWatchOnly:
            String(localized: "You will not be able to send bitcoin with this wallet. You will only be able to create receive addresses and view transactions.")
        case .errorImportingHotWallet:
            String(localized: "Unable to import this wallet. Please check the file or words and try again.")
        case .importedSuccessfully:
            String(localized: "Wallet imported successfully.")
        case .importedLabelsSuccessfully:
            String(localized: "Labels imported successfully.")
        case .unableToSelectWallet:
            String(localized: "Unable to select wallet. Please try again.")
        case .errorImportingHardwareWallet:
            String(localized: "Unable to import this hardware wallet. Please try again.")
        case .invalidFileFormat:
            String(localized: "This file is not in a format Cove can import.")
        case .invalidFormat:
            String(localized: "This data is not in a format Cove can import.")
        case let .addressWrongNetwork(address, network, currentNetwork):
            String(localized: "The address \(address.spacedOut()) is for \(network.localizedDisplayName), but your wallet is on \(currentNetwork.localizedDisplayName).")
        case .noWalletSelected:
            String(localized: "Please select a wallet to send to this address.")
        case let .foundAddress(address, _):
            String(localized: "Address: \(address.spacedOut())")
        case .noCameraPermission:
            String(localized: "Please allow camera access in Settings to use this feature.")
        case .failedToScanQr:
            String(localized: "Unable to read this QR code. Please try scanning it again.")
        case let .noUnsignedTransactionFound(txId):
            String(localized: "No unsigned transaction found for transaction \(txId.asHashString()).")
        case .unableToGetAddress:
            String(localized: "Unable to get an address for this wallet. Please try again.")
        case .cantSendOnWatchOnlyWallet:
            String(localized: "This wallet can only watch transactions. To send, import the seed words for this wallet or import the public key from your hardware wallet.")
        case .watchOnlyImportHardware:
            String(localized: "Choose how to import your hardware wallet.")
        case .watchOnlyImportWords:
            String(localized: "Choose how to import your seed words.")
        case .uninitializedTapSigner:
            String(localized: "This TAPSIGNER has not been set up yet. Would you like to set it up now?")
        case .tapSignerSetupFailed:
            String(localized: "Unable to set up this TAPSIGNER. Please try again.")
        case .tapSignerDeriveFailed:
            String(localized: "Unable to import this TAPSIGNER. Please try again.")
        case .tapSignerInvalidAuth, .tapSignerWrongPin:
            String(localized: "The PIN you entered was incorrect. Please try again.")
        case .tapSignerWalletFound:
            String(localized: "Would you like to go to this wallet?")
        case .initializedTapSigner:
            String(localized: "Would you like to start using this TAPSIGNER with Cove?")
        case .tapSignerNoBackup:
            String(localized: "You need to back up this wallet before changing the PIN. Would you like to take a backup now?")
        case .walletDatabaseCorrupted:
            String(localized: "The wallet database is corrupted and cannot be opened. You can delete this wallet and re-import it to fix the issue.")
        case let .general(_, message):
            message
        case .loading:
            ""
        }
    }
}
