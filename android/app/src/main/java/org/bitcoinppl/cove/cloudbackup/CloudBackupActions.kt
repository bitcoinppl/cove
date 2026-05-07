package org.bitcoinppl.cove.cloudbackup

import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupEnableContext
import org.bitcoinppl.cove_core.CloudBackupVerificationSource
import org.bitcoinppl.cove_core.SavedPasskeyConfirmationMode

internal fun manualEnableCloudBackup(
    source: CloudBackupVerificationSource,
): CloudBackupManagerAction =
    CloudBackupManagerAction.EnableCloudBackup(
        manualEnableContext(source),
    )

internal fun manualEnableCloudBackupNoDiscovery(
    source: CloudBackupVerificationSource,
): CloudBackupManagerAction =
    CloudBackupManagerAction.EnableCloudBackupNoDiscovery(
        manualEnableContext(source),
    )

internal fun enableCloudBackupForceNew(
    context: CloudBackupEnableContext,
): CloudBackupManagerAction =
    CloudBackupManagerAction.EnableCloudBackupForceNew(context)

private fun manualEnableContext(source: CloudBackupVerificationSource): CloudBackupEnableContext =
    CloudBackupEnableContext(
        SavedPasskeyConfirmationMode.MANUAL,
        source,
    )
