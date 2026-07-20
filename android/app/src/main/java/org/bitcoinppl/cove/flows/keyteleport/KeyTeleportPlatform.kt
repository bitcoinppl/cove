package org.bitcoinppl.cove.flows.keyteleport

import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.PersistableBundle
import org.bitcoinppl.cove_core.KeyTeleportAlert
import org.bitcoinppl.cove_core.MultiFormat
import org.bitcoinppl.cove_core.StringOrData

internal fun KeyTeleportManager.ingestKeyTeleportMultiFormat(multiFormat: MultiFormat): Boolean =
    when (multiFormat) {
        is MultiFormat.KeyTeleportReceiver -> {
            ingest(StringOrData.String(multiFormat.v1.bbqrPart()))
            true
        }

        is MultiFormat.KeyTeleportSender -> {
            ingest(StringOrData.String(multiFormat.v1.bbqrPart()))
            true
        }

        else -> false
    }

internal fun KeyTeleportAlert.messageForDisplay(): String =
    technicalMessageForDisplay()
        ?: userMessageForDisplay()

private fun KeyTeleportAlert.userMessageForDisplay(): String =
    when (this) {
        is KeyTeleportAlert.NoActiveReceiveSession -> "No active receive session was found."
        is KeyTeleportAlert.ReceiveSessionExpired -> "The receive session expired. Start a new session."
        is KeyTeleportAlert.ParseFailed -> "That is not a valid Key Teleport code."
        is KeyTeleportAlert.UnsupportedPsbt -> "PSBT teleport packets are not supported yet."
        is KeyTeleportAlert.UnsupportedPayload -> "This Key Teleport payload type is not supported yet."
        is KeyTeleportAlert.InvalidPayload -> "The transfer unlocked, but its contents are invalid."
        is KeyTeleportAlert.WrongReceiverCode -> "The receiver code is incorrect."
        is KeyTeleportAlert.WrongTeleportPassword -> "The sender password is incorrect."
        is KeyTeleportAlert.NoEligibleWallets -> "No eligible hot wallets are available on this device."
        is KeyTeleportAlert.IneligibleWallet -> "That wallet is not eligible for Key Teleport."
        is KeyTeleportAlert.NoPendingSend -> "There is no pending send in progress."
        is KeyTeleportAlert.NoPendingReceiveSecret -> "There is no received wallet to import."
        else -> error("technical Key Teleport alert must provide its own message")
    }

private fun KeyTeleportAlert.technicalMessageForDisplay(): String? =
    when (this) {
        is KeyTeleportAlert.ImportFailed -> v1
        is KeyTeleportAlert.Keychain -> v1
        is KeyTeleportAlert.Protocol -> v1
        is KeyTeleportAlert.Database -> v1
        else -> null
    }

internal fun readClipboardText(context: Context): String? {
    val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager

    return clipboard.primaryClip?.getItemAt(0)?.coerceToText(context)?.toString()
}

internal fun copyText(
    context: Context,
    label: String,
    text: String,
    sensitive: Boolean = false,
) {
    val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
    val clip = ClipData.newPlainText(label, text)
    if (sensitive && Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        clip.description.extras = PersistableBundle().apply {
            putBoolean(ClipDescription.EXTRA_IS_SENSITIVE, true)
        }
    }

    clipboard.setPrimaryClip(clip)
}

internal fun shareText(
    context: Context,
    title: String,
    text: String,
) {
    val intent =
        Intent(Intent.ACTION_SEND).apply {
            type = "text/plain"
            putExtra(Intent.EXTRA_TEXT, text)
        }

    context.startActivity(Intent.createChooser(intent, title))
}
