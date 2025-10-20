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
    data class DuplicateWallet(val walletId: WalletId) : AppAlertState()

    // errors
    data object InvalidWordGroup : AppAlertState()

    data class ErrorImportingHotWallet(val message: String) : AppAlertState()

    data class AddressWrongNetwork(
        val address: Address,
        val network: Network,
        val currentNetwork: Network,
    ) : AppAlertState()

    data class FoundAddress(val address: Address, val amount: Amount?) : AppAlertState()

    data object UnableToSelectWallet : AppAlertState()

    data class ErrorImportingHardwareWallet(val message: String) : AppAlertState()

    data class InvalidFileFormat(val message: String) : AppAlertState()

    data class NoWalletSelected(val address: Address) : AppAlertState()

    data class InvalidFormat(val message: String) : AppAlertState()

    data class NoUnsignedTransactionFound(val txId: TxId) : AppAlertState()

    data class UnableToGetAddress(val error: String) : AppAlertState()

    data object NoCameraPermission : AppAlertState()

    data class FailedToScanQr(val error: String) : AppAlertState()

    data object CantSendOnWatchOnlyWallet : AppAlertState()

    data class TapSignerSetupFailed(val message: String) : AppAlertState()

    data class TapSignerDeriveFailed(val message: String) : AppAlertState()

    data object TapSignerInvalidAuth : AppAlertState()

    data class TapSignerNoBackup(val tapSigner: TapSigner) : AppAlertState()

    // generic message or error
    data class General(val title: String, val message: String) : AppAlertState()

    // action
    data class UninitializedTapSigner(val tapSigner: TapSigner) : AppAlertState()

    data class TapSignerWalletFound(val walletId: WalletId) : AppAlertState()

    data class InitializedTapSigner(val tapSigner: TapSigner) : AppAlertState()

    fun title(): String {
        return when (this) {
            is InvalidWordGroup -> "Words Not Valid"
            is DuplicateWallet -> "Duplicate Wallet"
            is ErrorImportingHotWallet -> "Error"
            is ImportedSuccessfully, is ImportedLabelsSuccessfully -> "Success"
            is UnableToSelectWallet -> "Error"
            is ErrorImportingHardwareWallet -> "Error Importing Hardware Wallet"
            is InvalidFileFormat -> "Invalid File Format"
            is InvalidFormat -> "Invalid Format"
            is AddressWrongNetwork -> "Wrong Network"
            is NoWalletSelected, is FoundAddress -> "Found Address"
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
    }
}
