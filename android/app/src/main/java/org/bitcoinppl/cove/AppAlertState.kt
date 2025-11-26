package org.bitcoinppl.cove

import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.tapcard.*
import org.bitcoinppl.cove_core.types.*

/**
 * represents different alert states that can be shown in the app
 * ported from iOS AppAlertState.swift
 */
sealed class AppAlertState {
    // success
    data object ImportedSuccessfully : AppAlertState()

    data object ImportedLabelsSuccessfully : AppAlertState()

    // warn
    data class DuplicateWallet(
        val walletId: WalletId,
    ) : AppAlertState()

    // errors
    data object InvalidWordGroup : AppAlertState()

    data class ErrorImportingHotWallet(
        val message: String,
    ) : AppAlertState()

    data class AddressWrongNetwork(
        val address: Address,
        val network: Network,
        val currentNetwork: Network,
    ) : AppAlertState()

    data class FoundAddress(
        val address: Address,
        val amount: Amount?,
    ) : AppAlertState()

    data object UnableToSelectWallet : AppAlertState()

    data class ErrorImportingHardwareWallet(
        val message: String,
    ) : AppAlertState()

    data class InvalidFileFormat(
        val message: String,
    ) : AppAlertState()

    data class NoWalletSelected(
        val address: Address,
    ) : AppAlertState()

    data class InvalidFormat(
        val message: String,
    ) : AppAlertState()

    data class NoUnsignedTransactionFound(
        val txId: TxId,
    ) : AppAlertState()

    data class UnableToGetAddress(
        val error: String,
    ) : AppAlertState()

    data object NoCameraPermission : AppAlertState()

    data class FailedToScanQr(
        val error: String,
    ) : AppAlertState()

    data object CantSendOnWatchOnlyWallet : AppAlertState()

    data class TapSignerSetupFailed(
        val message: String,
    ) : AppAlertState()

    data class TapSignerDeriveFailed(
        val message: String,
    ) : AppAlertState()

    data object TapSignerInvalidAuth : AppAlertState()

    data class TapSignerNoBackup(
        val tapSigner: TapSigner,
    ) : AppAlertState()

    // generic message or error
    data class General(
        val title: String,
        val message: String,
    ) : AppAlertState()

    // action
    data class UninitializedTapSigner(
        val tapSigner: TapSigner,
    ) : AppAlertState()

    data class TapSignerWalletFound(
        val walletId: WalletId,
    ) : AppAlertState()

    data class InitializedTapSigner(
        val tapSigner: TapSigner,
    ) : AppAlertState()

    fun title(): String =
        when (this) {
            is InvalidWordGroup -> "Words Not Valid"
            is DuplicateWallet -> "Duplicate Wallet"
            is ErrorImportingHotWallet -> "Error"
            is ImportedSuccessfully, is ImportedLabelsSuccessfully -> "Success"
            is UnableToSelectWallet -> "Error"
            is ErrorImportingHardwareWallet -> "Error Importing Hardware Wallet"
            is InvalidFileFormat -> "Invalid File Format"
            is InvalidFormat -> "Invalid Format"
            is AddressWrongNetwork -> "Wrong Network"
            is NoWalletSelected -> "Select a Wallet"
            is FoundAddress -> "Found Address"
            is NoCameraPermission -> "Camera Access is Required"
            is FailedToScanQr -> "Failed to Scan QR"
            is NoUnsignedTransactionFound -> "No Unsigned Transaction Found"
            is UnableToGetAddress -> "Unable to Get Address"
            is CantSendOnWatchOnlyWallet -> "Watch Only Wallet"
            is UninitializedTapSigner -> "Setup TAPSIGNER?"
            is TapSignerSetupFailed -> "Setup Failed"
            is TapSignerDeriveFailed -> "TAPSIGNER Import Failed"
            is TapSignerInvalidAuth -> "Wrong PIN"
            is TapSignerWalletFound -> "Wallet Found"
            is InitializedTapSigner -> "Import TAPSIGNER?"
            is TapSignerNoBackup -> "No Backup Found"
            is General -> title
        }

    fun message(): String =
        when (this) {
            is InvalidWordGroup -> "The words do not create a valid wallet. Please check the words and try again."
            is DuplicateWallet -> "This wallet has already been imported! Taking you there now..."
            is ErrorImportingHotWallet -> message
            is ImportedSuccessfully -> "Wallet imported successfully"
            is ImportedLabelsSuccessfully -> "Labels imported successfully"
            is UnableToSelectWallet -> "Unable to select wallet, please try again"
            is ErrorImportingHardwareWallet -> message
            is InvalidFileFormat -> message
            is InvalidFormat -> message
            is AddressWrongNetwork -> "This address is for ${network.name}, but you're on ${currentNetwork.name}"
            is NoWalletSelected -> "Please select a wallet to send to this address"
            is FoundAddress -> "Address: ${address.spacedOut()}"
            is NoCameraPermission -> "Please enable camera access in settings to scan QR codes"
            is FailedToScanQr -> error
            is NoUnsignedTransactionFound -> "No unsigned transaction found for this transaction"
            is UnableToGetAddress -> error
            is CantSendOnWatchOnlyWallet -> "You cannot send from a watch-only wallet"
            is UninitializedTapSigner -> "This TAPSIGNER needs to be set up before it can be used"
            is TapSignerSetupFailed -> message
            is TapSignerDeriveFailed -> message
            is TapSignerInvalidAuth -> "The PIN you entered is incorrect"
            is TapSignerWalletFound -> "A wallet for this TAPSIGNER was found"
            is InitializedTapSigner -> "Would you like to import this TAPSIGNER?"
            is TapSignerNoBackup -> "No backup found for this TAPSIGNER"
            is General -> message
        }

    fun isSnackbar(): Boolean =
        when (this) {
            is ImportedSuccessfully,
            is ImportedLabelsSuccessfully,
            is InvalidWordGroup,
            is ErrorImportingHotWallet,
            is UnableToSelectWallet,
            is ErrorImportingHardwareWallet,
            is InvalidFileFormat,
            is InvalidFormat,
            is NoUnsignedTransactionFound,
            is UnableToGetAddress,
            is FailedToScanQr,
            is CantSendOnWatchOnlyWallet,
            is TapSignerSetupFailed,
            is TapSignerDeriveFailed,
            is TapSignerInvalidAuth,
            -> true
            else -> false
        }
}
