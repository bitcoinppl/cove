@file:Suppress("FunctionNaming", "PackageNaming")

package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import org.bitcoinppl.cove.views.lockaware.AlertDialog
import org.bitcoinppl.cove_core.SecurityAlertState

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
    // Sync Now uploads only wallets, so neither message sends the user there
    CloudBackupKeyMissing(
        "Cove couldn't confirm the key for your cloud backup, so it can't restore your " +
            "wallets after a wipe. Open Cloud Backup and run Verify to fix it, then you can set " +
            "the Wipe Data PIN. Or back up each wallet's recovery words first.",
    ),
    // only iCloud Drive reports an upload as pending, so Android never shows this; kept
    // so every state Rust can send has an alert
    CloudBackupKeyUploading(
        "iCloud hasn't finished uploading the key for your cloud backup, so it can't restore " +
            "your wallets after a wipe yet. Check that you're online and iCloud Drive is on for " +
            "Cove, then try again. Or back up each wallet's recovery words first.",
    ),
    ;

    companion object {
        /** The refusal [state] shows, or null when it names a wallet or isn't a refusal */
        fun of(state: SecurityAlertState): WipeDataPinRefusal? =
            when (state) {
                SecurityAlertState.UnverifiedDecoyWallets -> UnverifiedDecoyWallets
                SecurityAlertState.UnreadableWallets -> UnreadableWallets
                SecurityAlertState.CloudBackupKeyMissing -> CloudBackupKeyMissing
                SecurityAlertState.CloudBackupKeyUploading -> CloudBackupKeyUploading
                else -> null
            }
    }
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
