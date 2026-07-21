package org.bitcoinppl.cove.cloudbackup

internal data class CloudBackupScreenActions(
    val onBack: () -> Unit,
    val onRecreate: () -> Unit,
    val onReinitialize: () -> Unit,
    val onSwitchAccount: () -> Unit = {},
)
