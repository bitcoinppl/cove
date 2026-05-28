package org.bitcoinppl.cove.cloudbackup

import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupEnableContext
import org.bitcoinppl.cove_core.CloudBackupVerificationSource
import org.bitcoinppl.cove_core.SavedPasskeyConfirmationMode

internal fun manualEnableCloudBackupNoDiscovery(
    source: CloudBackupVerificationSource,
): CloudBackupManagerAction =
    CloudBackupManagerAction.EnableCloudBackupNoDiscovery(
        manualEnableContext(source),
    )

private fun manualEnableContext(source: CloudBackupVerificationSource): CloudBackupEnableContext =
    CloudBackupEnableContext(
        SavedPasskeyConfirmationMode.MANUAL,
        source,
    )
