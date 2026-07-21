@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.SettingsFlow

import org.bitcoinppl.cove_core.CloudBackupSettingsRowStatus

internal fun shouldShowCloudBackupSettings(
    isInDecoyMode: Boolean,
): Boolean = !isInDecoyMode

internal enum class CloudBackupSettingsSeverity {
    NEUTRAL,
    INFO,
    SUCCESS,
    WARNING,
    ERROR,
}

internal fun cloudBackupSettingsSeverity(
    status: CloudBackupSettingsRowStatus,
): CloudBackupSettingsSeverity =
    when (status) {
        is CloudBackupSettingsRowStatus.Disabled -> CloudBackupSettingsSeverity.NEUTRAL
        is CloudBackupSettingsRowStatus.Disabling,
        is CloudBackupSettingsRowStatus.SettingUp,
        is CloudBackupSettingsRowStatus.Restoring,
        is CloudBackupSettingsRowStatus.Confirming,
        is CloudBackupSettingsRowStatus.CheckingSync,
        is CloudBackupSettingsRowStatus.Syncing,
        -> CloudBackupSettingsSeverity.INFO
        is CloudBackupSettingsRowStatus.Active -> CloudBackupSettingsSeverity.SUCCESS
        is CloudBackupSettingsRowStatus.PasskeyMissing,
        is CloudBackupSettingsRowStatus.PasskeyProviderUnsupported,
        is CloudBackupSettingsRowStatus.Unverified,
        is CloudBackupSettingsRowStatus.VerificationRecommended,
        is CloudBackupSettingsRowStatus.NoFiles,
        is CloudBackupSettingsRowStatus.DriveUnavailable,
        is CloudBackupSettingsRowStatus.AuthorizationRequired,
        is CloudBackupSettingsRowStatus.RecoveryRequired,
        -> CloudBackupSettingsSeverity.WARNING
        is CloudBackupSettingsRowStatus.Error -> CloudBackupSettingsSeverity.ERROR
    }
