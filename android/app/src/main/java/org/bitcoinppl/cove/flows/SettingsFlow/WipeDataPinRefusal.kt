@file:Suppress("FunctionNaming", "PackageNaming")

package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import org.bitcoinppl.cove.views.lockaware.AlertDialog

/** Why Rust kept the Wipe Data PIN off when no single wallet can be named */
internal enum class WipeDataPinRefusal(
    val message: String,
) {
    UnverifiedDecoyWallets(
        "A decoy wallet has not been backed up. Unlock Cove with your decoy PIN and back up " +
            "that wallet before enabling the Wipe Data PIN. If the decoy PIN is off, set a new " +
            "one in Settings first. If you wipe the data without having " +
            "a backup of your wallet, you will lose the bitcoin in that wallet.",
    ),
    UnreadableWallets(
        "Cove couldn't read your wallets to check that they're backed up, so the Wipe Data " +
            "PIN stays off. Please try again.",
    ),
    CloudBackupKeyUnconfirmed(
        "Cove couldn't confirm the key for your cloud backup, so it can't restore your wallets " +
            "after a wipe. Sync Cloud Backup, or back up each wallet's recovery words, before " +
            "enabling the Wipe Data PIN.",
    ),
}

@Composable
internal fun WipeDataPinRefusedAlert(
    refusal: WipeDataPinRefusal,
    callbacks: SecurityAlertDialogCallbacks,
) {
    AlertDialog(
        onDismissRequest = callbacks.onDismiss,
        title = { Text("Can't Enable Wipe Data PIN") },
        text = { Text(refusal.message) },
        confirmButton = {
            TextButton(onClick = callbacks.onDismiss) {
                Text("OK")
            }
        },
    )
}
