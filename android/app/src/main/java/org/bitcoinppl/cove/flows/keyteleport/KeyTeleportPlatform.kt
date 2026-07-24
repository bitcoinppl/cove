package org.bitcoinppl.cove.flows.keyteleport

import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.PersistableBundle
import org.bitcoinppl.cove_core.KeyTeleportAlert
import org.bitcoinppl.cove_core.KeyTeleportInput
import org.bitcoinppl.cove_core.MultiFormat
import org.bitcoinppl.cove_core.StringOrData
import java.util.UUID

private const val SENSITIVE_CLIPBOARD_TOKEN = "org.bitcoinppl.cove.keyteleport.CLIP_TOKEN"
private const val SENSITIVE_CLIPBOARD_LIFETIME_MILLIS = 60_000L

internal fun KeyTeleportManager.ingestKeyTeleportMultiFormat(
    multiFormat: MultiFormat,
    direction: KeyTeleportFlowDirection,
): Boolean =
    when (multiFormat) {
        is MultiFormat.KeyTeleportReceiver -> {
            if (direction == KeyTeleportFlowDirection.SEND) {
                ingest(KeyTeleportInput.Receiver(multiFormat.v1), direction)
            } else {
                multiFormat.destroy()
                false
            }
        }

        is MultiFormat.KeyTeleportSender -> {
            if (direction == KeyTeleportFlowDirection.RECEIVE) {
                ingest(KeyTeleportInput.Sender(multiFormat.v1), direction)
            } else {
                multiFormat.destroy()
                false
            }
        }

        else -> {
            multiFormat.destroy()
            false
        }
    }

internal fun KeyTeleportManager.ingestKeyTeleportText(
    text: String,
    direction: KeyTeleportFlowDirection,
): Boolean {
    val multiFormat =
        runCatching {
            StringOrData.String(text).tryIntoMultiFormat()
        }.getOrNull() ?: return false

    return ingestKeyTeleportMultiFormat(multiFormat, direction)
}

internal fun KeyTeleportAlert.messageForDisplay(): String =
    receiveMessageForDisplay()
        ?: nonReceiveMessageForDisplay()

private fun KeyTeleportAlert.receiveMessageForDisplay(): String? =
    when (this) {
        is KeyTeleportAlert.NoActiveReceiveSession -> "No active receive session was found."
        is KeyTeleportAlert.ReceiveSessionExpired -> "The receive session expired. Start a new session."
        is KeyTeleportAlert.ReceiveSessionReset ->
            "The previous receive request was unreadable, so Cove replaced it. Responses for the old request will not work."
        is KeyTeleportAlert.ReceiveSessionScopeChanged ->
            "The receive request no longer matches the selected network or wallet mode. Start a new receive session."
        is KeyTeleportAlert.WrongTeleportPassword -> "The sender password is incorrect."
        is KeyTeleportAlert.NoPendingReceiveSecret -> "There is no received wallet to import."
        else -> null
    }

private fun KeyTeleportAlert.nonReceiveMessageForDisplay(): String =
    when (this) {
        is KeyTeleportAlert.ParseFailed -> "That is not a valid KeyTeleport code."
        is KeyTeleportAlert.ConflictingTransferDirection ->
            "This packet conflicts with the active KeyTeleport transfer. Finish the current flow first."
        is KeyTeleportAlert.UnsupportedPsbt -> "PSBT teleport packets are not supported yet."
        is KeyTeleportAlert.UnsupportedPayload -> "This KeyTeleport payload type is not supported yet."
        is KeyTeleportAlert.InvalidPayload -> "The transfer unlocked, but its contents are invalid."
        is KeyTeleportAlert.WrongReceiverCode -> "The receiver code is incorrect."
        is KeyTeleportAlert.NoEligibleWallets -> "No eligible hot wallets are available on this device."
        is KeyTeleportAlert.IneligibleWallet -> "That wallet is not eligible for KeyTeleport."
        is KeyTeleportAlert.NoPendingSend -> "There is no pending send in progress."
        is KeyTeleportAlert.ImportFailed -> v1
        is KeyTeleportAlert.Keychain -> v1
        is KeyTeleportAlert.Protocol -> v1
        is KeyTeleportAlert.Database -> v1
        else -> "Something went wrong with KeyTeleport."
    }

internal fun readClipboardText(context: Context): String? {
    val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
    val clip = clipboard.primaryClip

    return clip
        ?.takeIf { it.itemCount > 0 }
        ?.getItemAt(0)
        ?.coerceToText(context)
        ?.toString()
}

internal fun copyText(
    context: Context,
    label: String,
    text: String,
    sensitive: Boolean = false,
) {
    val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
    val clip = ClipData.newPlainText(label, text)
    if (sensitive) {
        val token = UUID.randomUUID().toString()
        clip.description.extras = PersistableBundle().apply {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                putBoolean(ClipDescription.EXTRA_IS_SENSITIVE, true)
            }
            putString(SENSITIVE_CLIPBOARD_TOKEN, token)
        }
        clipboard.setPrimaryClip(clip)
        SensitiveClipboardExpiry.schedule(context.applicationContext, token)
    } else {
        clipboard.setPrimaryClip(clip)
    }
}

internal fun clearOwnedSensitiveClipboard(context: Context) {
    SensitiveClipboardExpiry.clearIfOwned(context.applicationContext)
}

private object SensitiveClipboardExpiry {
    private val handler = Handler(Looper.getMainLooper())
    private var pendingClear: Runnable? = null
    private var currentToken: String? = null

    fun schedule(
        context: Context,
        token: String,
    ) {
        pendingClear?.let(handler::removeCallbacks)
        currentToken = token
        pendingClear =
            Runnable {
                clearIfOwned(context)
            }.also {
                handler.postDelayed(it, SENSITIVE_CLIPBOARD_LIFETIME_MILLIS)
            }
    }

    fun clearIfOwned(context: Context) {
        val expectedToken = currentToken ?: return
        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        val token =
            runCatching {
                clipboard.primaryClipDescription?.extras?.getString(SENSITIVE_CLIPBOARD_TOKEN)
            }.getOrNull()

        if (token == expectedToken) {
            runCatching(clipboard::clearPrimaryClip)
        }

        pendingClear?.let(handler::removeCallbacks)
        pendingClear = null
        currentToken = null
    }
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
