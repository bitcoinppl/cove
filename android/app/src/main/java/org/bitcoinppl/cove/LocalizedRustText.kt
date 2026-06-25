package org.bitcoinppl.cove

import org.bitcoinppl.cove_core.AfterPinAction
import org.bitcoinppl.cove_core.AppAlertState
import org.bitcoinppl.cove_core.CatastrophicCloudRestoreInconclusiveReason
import org.bitcoinppl.cove_core.CatastrophicCloudRestoreProvider
import org.bitcoinppl.cove_core.CatastrophicCloudRestoreResult
import org.bitcoinppl.cove_core.CloudBackupDestructiveOperationState
import org.bitcoinppl.cove_core.CloudBackupFailure
import org.bitcoinppl.cove_core.CloudCheckIssue
import org.bitcoinppl.cove_core.DeepVerificationFailure
import org.bitcoinppl.cove_core.OnboardingError
import org.bitcoinppl.cove_core.OnboardingRestoreFailure
import org.bitcoinppl.cove_core.PinUpdateFailure
import org.bitcoinppl.cove_core.ScanProgress
import org.bitcoinppl.cove_core.SendFlowAlertState
import org.bitcoinppl.cove_core.SendFlowException
import org.bitcoinppl.cove_core.WalletAddressType
import org.bitcoinppl.cove_core.WalletSecretType
import org.bitcoinppl.cove_core.WalletType
import org.bitcoinppl.cove_core.types.Network

fun Network.localizedDisplayText(): UiText =
    when (this) {
        Network.BITCOIN -> UiText.resource(R.string.network_bitcoin)
        Network.TESTNET -> UiText.resource(R.string.network_testnet)
        Network.TESTNET4 -> UiText.resource(R.string.network_testnet4)
        Network.SIGNET -> UiText.resource(R.string.network_signet)
    }

fun WalletType.localizedDisplayText(): UiText =
    when (this) {
        WalletType.HOT -> UiText.resource(R.string.wallet_type_hot)
        WalletType.COLD -> UiText.resource(R.string.wallet_type_cold)
        WalletType.XPUB_ONLY -> UiText.resource(R.string.wallet_type_xpub_only)
        WalletType.WATCH_ONLY -> UiText.resource(R.string.wallet_type_watch_only)
    }

fun WalletSecretType.localizedDisplayText(): UiText =
    when (this) {
        WalletSecretType.MNEMONIC -> UiText.resource(R.string.wallet_secret_mnemonic)
        WalletSecretType.TAP_SIGNER_BACKUP -> UiText.resource(R.string.wallet_secret_tap_signer_backup)
        WalletSecretType.NONE -> UiText.resource(R.string.wallet_secret_none)
        WalletSecretType.UNKNOWN -> UiText.resource(R.string.wallet_secret_unknown)
    }

fun WalletAddressType.localizedDisplayText(): UiText =
    when (this) {
        WalletAddressType.NATIVE_SEGWIT -> UiText.resource(R.string.wallet_address_type_native_segwit)
        WalletAddressType.WRAPPED_SEGWIT -> UiText.resource(R.string.wallet_address_type_wrapped_segwit)
        WalletAddressType.LEGACY -> UiText.resource(R.string.wallet_address_type_legacy)
    }

fun CatastrophicCloudRestoreResult.localizedFailureMessage(): UiText? =
    when (this) {
        CatastrophicCloudRestoreResult.BackupFound -> null
        is CatastrophicCloudRestoreResult.NoBackupFound -> UiText.resource(
            R.string.common_remaining_no_cloud_backup_found,
            provider.localizedAccountName(),
        )
        is CatastrophicCloudRestoreResult.Offline -> UiText.resource(
            R.string.common_remaining_cannot_check_cloud_backup_offline,
            provider.localizedStorageName(),
        )
        CatastrophicCloudRestoreResult.Unreadable -> UiText.resource(
            R.string.common_remaining_cloud_backup_unreadable,
        )
        is CatastrophicCloudRestoreResult.Inconclusive -> reason.localizedMessage(provider)
    }

private fun CatastrophicCloudRestoreProvider.localizedStorageName(): UiText =
    when (this) {
        CatastrophicCloudRestoreProvider.I_CLOUD_DRIVE -> UiText.resource(R.string.common_remaining_icloud)
        CatastrophicCloudRestoreProvider.GOOGLE_DRIVE -> UiText.resource(R.string.common_remaining_google_drive)
    }

private fun CatastrophicCloudRestoreProvider.localizedAccountName(): UiText =
    when (this) {
        CatastrophicCloudRestoreProvider.I_CLOUD_DRIVE -> UiText.resource(R.string.common_remaining_icloud_account)
        CatastrophicCloudRestoreProvider.GOOGLE_DRIVE -> UiText.resource(R.string.common_remaining_google_account)
    }

private fun CatastrophicCloudRestoreInconclusiveReason.localizedMessage(
    provider: CatastrophicCloudRestoreProvider,
): UiText =
    when (this) {
        CatastrophicCloudRestoreInconclusiveReason.AUTHORIZATION_REQUIRED -> UiText.resource(
            R.string.common_remaining_cloud_backup_authorization_required,
            provider.localizedStorageName(),
        )
        CatastrophicCloudRestoreInconclusiveReason.QUOTA_EXCEEDED -> UiText.resource(
            R.string.common_remaining_cloud_backup_quota_exceeded,
            provider.localizedStorageName(),
        )
        CatastrophicCloudRestoreInconclusiveReason.PROVIDER_UNAVAILABLE -> UiText.resource(
            R.string.common_remaining_cloud_backup_provider_unavailable,
            provider.localizedStorageName(),
        )
        CatastrophicCloudRestoreInconclusiveReason.UNKNOWN -> UiText.resource(
            R.string.common_remaining_cloud_backup_check_failed,
        )
    }

fun CloudCheckIssue.localizedMessage(): UiText =
    when (this) {
        CloudCheckIssue.OFFLINE -> UiText.resource(R.string.cloud_restore_issue_offline)
        CloudCheckIssue.CLOUD_UNAVAILABLE -> UiText.resource(R.string.cloud_restore_issue_cloud_unavailable)
        CloudCheckIssue.UNKNOWN -> UiText.resource(R.string.cloud_restore_issue_unknown)
    }

fun OnboardingRestoreFailure.localizedMessage(): UiText =
    when (this) {
        OnboardingRestoreFailure.TIMED_OUT -> UiText.resource(R.string.onboarding_restore_failure_timed_out)
        OnboardingRestoreFailure.FAILED -> UiText.resource(R.string.onboarding_restore_failure_failed)
    }

fun OnboardingError.localizedMessage(): UiText =
    when (this) {
        OnboardingError.WALLET_CREATION_FAILED -> UiText.resource(R.string.onboarding_error_wallet_creation_failed)
        OnboardingError.COMPLETION_FAILED -> UiText.resource(R.string.onboarding_error_completion_failed)
    }

fun PinUpdateFailure.localizedMessage(): UiText =
    when (this) {
        PinUpdateFailure.UPDATE_FAILED -> UiText.resource(R.string.settings_security_update_pin_error)
        PinUpdateFailure.SAME_AS_WIPE_DATA_PIN -> UiText.resource(
            R.string.settings_security_update_pin_same_as_wipe_data_pin,
        )
        PinUpdateFailure.SAME_AS_DECOY_PIN -> UiText.resource(
            R.string.settings_security_update_pin_same_as_decoy_pin,
        )
    }

fun AfterPinAction.localizedUserMessage(): UiText =
    when (this) {
        is AfterPinAction.Derive -> UiText.resource(R.string.after_pin_derive)
        is AfterPinAction.Change -> UiText.resource(R.string.after_pin_change)
        is AfterPinAction.Backup -> UiText.resource(R.string.after_pin_backup)
        is AfterPinAction.Sign -> UiText.resource(R.string.after_pin_sign)
    }

fun ScanProgress.localizedDisplayText(): UiText =
    when (this) {
        is ScanProgress.Bbqr -> UiText.resource(
            R.string.scan_progress_bbqr,
            scanned.toInt(),
            total.toInt(),
        )
        is ScanProgress.Ur -> UiText.resource(
            R.string.scan_progress_ur,
            (percentage * 100.0).toInt(),
        )
    }

fun ScanProgress.localizedDetailText(): UiText? =
    when (this) {
        is ScanProgress.Bbqr -> {
            val remaining = if (total > scanned) total - scanned else 0u
            if (remaining == 1u) {
                UiText.resource(R.string.scan_progress_one_part_left)
            } else {
                UiText.resource(R.string.scan_progress_parts_left, remaining.toInt())
            }
        }
        is ScanProgress.Ur -> null
    }

fun DeepVerificationFailure.localizedMessage(): UiText =
    when (this) {
        is DeepVerificationFailure.Retry -> UiText.resource(R.string.deep_verification_retry)
        is DeepVerificationFailure.RecreateManifest -> UiText.resource(R.string.deep_verification_recreate_manifest)
        is DeepVerificationFailure.ReinitializeBackup -> UiText.resource(R.string.deep_verification_reinitialize_backup)
        is DeepVerificationFailure.UnsupportedVersion -> UiText.resource(R.string.deep_verification_unsupported_version)
    }

fun DeepVerificationFailure.localizedWarning(): UiText? =
    when (this) {
        is DeepVerificationFailure.Retry,
        is DeepVerificationFailure.UnsupportedVersion,
        -> null
        is DeepVerificationFailure.RecreateManifest -> UiText.resource(R.string.deep_verification_recreate_manifest_warning)
        is DeepVerificationFailure.ReinitializeBackup -> UiText.resource(R.string.deep_verification_reinitialize_backup_warning)
    }

fun CloudBackupFailure.localizedMessage(): UiText =
    UiText.resource(R.string.cloud_backup_lifecycle_failed)

fun CloudBackupDestructiveOperationState.DisableFailed.localizedMessage(): UiText =
    UiText.resource(R.string.cloud_backup_disable_failed)

fun SendFlowException.localizedTitle(): UiText =
    when (this) {
        is SendFlowException.EmptyAddress,
        is SendFlowException.InvalidAddress,
        is SendFlowException.WrongNetwork,
        -> UiText.resource(R.string.send_alert_invalid_address)
        is SendFlowException.InvalidNumber,
        is SendFlowException.ZeroAmount,
        -> UiText.resource(R.string.send_alert_invalid_amount)
        is SendFlowException.InsufficientFunds,
        is SendFlowException.NoBalance,
        -> UiText.resource(R.string.send_alert_insufficient_funds)
        is SendFlowException.SendAmountToLow -> UiText.resource(R.string.send_alert_send_amount_too_low)
        is SendFlowException.UnableToGetFeeRate -> UiText.resource(R.string.send_alert_unable_get_fee_rate)
        is SendFlowException.UnableToBuildTxn -> UiText.resource(R.string.send_alert_unable_build_transaction)
        is SendFlowException.UnableToGetMaxSend -> UiText.resource(R.string.send_alert_unable_get_max_send)
        is SendFlowException.UnableToSaveUnsignedTransaction -> UiText.resource(R.string.send_alert_unable_save_unsigned_transaction)
        is SendFlowException.WalletManager -> UiText.resource(R.string.send_alert_error)
        is SendFlowException.UnableToGetFeeDetails -> UiText.resource(R.string.send_alert_fee_details_error)
    }

fun SendFlowException.localizedMessage(): UiText =
    when (this) {
        is SendFlowException.EmptyAddress -> UiText.resource(R.string.send_message_enter_address)
        is SendFlowException.InvalidNumber -> UiText.resource(R.string.send_message_valid_amount)
        is SendFlowException.ZeroAmount -> UiText.resource(R.string.send_message_zero_amount)
        is SendFlowException.NoBalance -> UiText.resource(R.string.send_message_no_balance)
        is SendFlowException.InvalidAddress -> UiText.resource(R.string.send_message_invalid_address, v1)
        is SendFlowException.WrongNetwork -> UiText.resource(
            R.string.send_message_wrong_network,
            address,
            validFor.localizedDisplayText(),
            current.localizedDisplayText(),
        )
        is SendFlowException.InsufficientFunds -> UiText.resource(R.string.send_message_insufficient_funds)
        is SendFlowException.SendAmountToLow -> UiText.resource(R.string.send_message_amount_too_low)
        is SendFlowException.UnableToGetFeeRate -> UiText.resource(R.string.send_message_get_fee_rate)
        is SendFlowException.WalletManager -> UiText.resource(R.string.send_message_wallet_manager)
        is SendFlowException.UnableToGetFeeDetails -> UiText.resource(R.string.send_message_fee_details)
        is SendFlowException.UnableToBuildTxn -> UiText.resource(R.string.send_message_build_transaction)
        is SendFlowException.UnableToGetMaxSend -> UiText.resource(R.string.send_message_get_max_send)
        is SendFlowException.UnableToSaveUnsignedTransaction -> UiText.resource(R.string.send_message_save_unsigned_transaction)
    }

fun SendFlowAlertState.localizedTitle(): UiText =
    when (this) {
        is SendFlowAlertState.Error -> v1.localizedTitle()
        is SendFlowAlertState.General -> UiText.raw(title)
        is SendFlowAlertState.UnableToLoadFees -> UiText.resource(R.string.send_alert_unable_load_fees)
        is SendFlowAlertState.FeeTooHigh -> UiText.resource(R.string.send_alert_fee_too_high)
        is SendFlowAlertState.HighFeeWarning -> UiText.resource(R.string.send_alert_high_fee_warning)
        is SendFlowAlertState.UnableToReadLockedCoins -> UiText.resource(R.string.send_alert_unable_read_locked_coins)
        is SendFlowAlertState.BalanceStillLoading -> UiText.resource(R.string.send_alert_balance_still_loading)
    }

fun SendFlowAlertState.localizedMessage(): UiText =
    when (this) {
        is SendFlowAlertState.Error -> v1.localizedMessage()
        is SendFlowAlertState.General -> UiText.raw(message)
        is SendFlowAlertState.UnableToLoadFees -> UiText.resource(R.string.send_message_unable_load_fees)
        is SendFlowAlertState.FeeTooHigh -> UiText.resource(R.string.send_message_fee_too_high)
        is SendFlowAlertState.HighFeeWarning -> UiText.resource(R.string.send_message_high_fee_warning)
        is SendFlowAlertState.UnableToReadLockedCoins -> UiText.resource(R.string.send_message_unable_read_locked_coins)
        is SendFlowAlertState.BalanceStillLoading -> UiText.resource(R.string.send_message_balance_still_loading)
    }

fun AppAlertState.localizedTitle(): UiText =
    when (this) {
        is AppAlertState.InvalidWordGroup -> UiText.resource(R.string.app_alert_words_not_valid_title)
        is AppAlertState.DuplicateWallet -> UiText.resource(R.string.app_alert_duplicate_wallet_title)
        is AppAlertState.HotWalletKeyMissing -> UiText.resource(R.string.app_alert_wallet_needs_recovery_title)
        is AppAlertState.ErrorImportingHotWallet -> UiText.resource(R.string.app_alert_error_title)
        is AppAlertState.ImportedSuccessfully,
        is AppAlertState.ImportedLabelsSuccessfully,
        -> UiText.resource(R.string.app_alert_success_title)
        is AppAlertState.UnableToSelectWallet -> UiText.resource(R.string.app_alert_error_title)
        is AppAlertState.ErrorImportingHardwareWallet -> UiText.resource(R.string.app_alert_error_importing_hardware_title)
        is AppAlertState.InvalidFileFormat -> UiText.resource(R.string.app_alert_invalid_file_format_title)
        is AppAlertState.InvalidFormat -> UiText.resource(R.string.app_alert_invalid_format_title)
        is AppAlertState.AddressWrongNetwork -> UiText.resource(R.string.app_alert_wrong_network_title)
        is AppAlertState.NoWalletSelected -> UiText.resource(R.string.app_alert_select_wallet_title)
        is AppAlertState.FoundAddress -> UiText.resource(R.string.app_alert_found_address_title)
        is AppAlertState.NoCameraPermission -> UiText.resource(R.string.app_alert_camera_required_title)
        is AppAlertState.FailedToScanQr -> UiText.resource(R.string.app_alert_failed_scan_qr_title)
        is AppAlertState.NoUnsignedTransactionFound -> UiText.resource(R.string.app_alert_no_unsigned_transaction_title)
        is AppAlertState.UnableToGetAddress -> UiText.resource(R.string.app_alert_unable_get_address_title)
        is AppAlertState.CantSendOnWatchOnlyWallet,
        is AppAlertState.ConfirmWatchOnly,
        -> UiText.resource(R.string.app_alert_watch_only_title)
        is AppAlertState.WatchOnlyImportHardware -> UiText.resource(R.string.app_alert_import_hardware_title)
        is AppAlertState.WatchOnlyImportWords -> UiText.resource(R.string.app_alert_import_words_title)
        is AppAlertState.UninitializedTapSigner -> UiText.resource(R.string.app_alert_setup_tapsigner_title)
        is AppAlertState.TapSignerSetupFailed -> UiText.resource(R.string.app_alert_setup_failed_title)
        is AppAlertState.TapSignerDeriveFailed -> UiText.resource(R.string.app_alert_tapsigner_import_failed_title)
        is AppAlertState.TapSignerInvalidAuth,
        is AppAlertState.TapSignerWrongPin,
        -> UiText.resource(R.string.app_alert_wrong_pin_title)
        is AppAlertState.TapSignerWalletFound -> UiText.resource(R.string.app_alert_wallet_found_title)
        is AppAlertState.InitializedTapSigner -> UiText.resource(R.string.app_alert_import_tapsigner_title)
        is AppAlertState.TapSignerNoBackup -> UiText.resource(R.string.app_alert_no_backup_found_title)
        is AppAlertState.WalletDatabaseCorrupted -> UiText.resource(R.string.app_alert_wallet_database_error_title)
        is AppAlertState.General -> UiText.raw(title)
        is AppAlertState.Loading -> UiText.resource(R.string.app_alert_loading_title)
    }

fun AppAlertState.localizedMessage(): UiText =
    when (this) {
        is AppAlertState.InvalidWordGroup -> UiText.resource(R.string.app_alert_words_not_valid_message)
        is AppAlertState.DuplicateWallet -> UiText.resource(R.string.app_alert_duplicate_wallet_message)
        is AppAlertState.HotWalletKeyMissing -> UiText.resource(R.string.app_alert_hot_wallet_key_missing_message)
        is AppAlertState.ConfirmWatchOnly -> UiText.resource(R.string.app_alert_confirm_watch_only_message)
        is AppAlertState.ErrorImportingHotWallet -> UiText.resource(R.string.app_alert_error_importing_wallet_message)
        is AppAlertState.ImportedSuccessfully -> UiText.resource(R.string.app_alert_imported_successfully_message)
        is AppAlertState.ImportedLabelsSuccessfully -> UiText.resource(R.string.app_alert_labels_imported_successfully_message)
        is AppAlertState.UnableToSelectWallet -> UiText.resource(R.string.app_alert_unable_select_wallet_message)
        is AppAlertState.ErrorImportingHardwareWallet -> UiText.resource(R.string.app_alert_error_importing_hardware_message)
        is AppAlertState.InvalidFileFormat -> UiText.resource(R.string.app_alert_invalid_file_format_message)
        is AppAlertState.InvalidFormat -> UiText.resource(R.string.app_alert_invalid_format_message)
        is AppAlertState.AddressWrongNetwork -> UiText.resource(
            R.string.app_alert_wrong_network_message,
            address.toString(),
            network.localizedDisplayText(),
            currentNetwork.localizedDisplayText(),
        )
        is AppAlertState.NoWalletSelected -> UiText.resource(R.string.app_alert_no_wallet_selected_message)
        is AppAlertState.FoundAddress -> UiText.resource(R.string.app_alert_found_address_message, address.spacedOut())
        is AppAlertState.NoCameraPermission -> UiText.resource(R.string.app_alert_camera_required_message)
        is AppAlertState.FailedToScanQr -> UiText.resource(R.string.app_alert_failed_scan_qr_message)
        is AppAlertState.NoUnsignedTransactionFound -> UiText.resource(R.string.app_alert_no_unsigned_transaction_message, txId.asHashString())
        is AppAlertState.UnableToGetAddress -> UiText.resource(R.string.app_alert_unable_get_address_message)
        is AppAlertState.CantSendOnWatchOnlyWallet -> UiText.resource(R.string.app_alert_watch_only_message)
        is AppAlertState.WatchOnlyImportHardware -> UiText.resource(R.string.app_alert_watch_only_import_hardware_message)
        is AppAlertState.WatchOnlyImportWords -> UiText.resource(R.string.app_alert_watch_only_import_words_message)
        is AppAlertState.UninitializedTapSigner -> UiText.resource(R.string.app_alert_uninitialized_tapsigner_message)
        is AppAlertState.TapSignerSetupFailed -> UiText.resource(R.string.app_alert_tapsigner_setup_failed_message)
        is AppAlertState.TapSignerDeriveFailed -> UiText.resource(R.string.app_alert_tapsigner_derive_failed_message)
        is AppAlertState.TapSignerInvalidAuth,
        is AppAlertState.TapSignerWrongPin,
        -> UiText.resource(R.string.app_alert_wrong_pin_message)
        is AppAlertState.TapSignerWalletFound -> UiText.resource(R.string.app_alert_tapsigner_wallet_found_message)
        is AppAlertState.InitializedTapSigner -> UiText.resource(R.string.app_alert_initialized_tapsigner_message)
        is AppAlertState.TapSignerNoBackup -> UiText.resource(R.string.app_alert_tapsigner_no_backup_message)
        is AppAlertState.WalletDatabaseCorrupted -> UiText.resource(R.string.app_alert_wallet_database_corrupted_message)
        is AppAlertState.General -> UiText.raw(message)
        is AppAlertState.Loading -> UiText.raw("")
    }
