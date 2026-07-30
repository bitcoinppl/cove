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
import android.os.SystemClock
import android.util.Log
import org.bitcoinppl.cove_core.KeyTeleportAlert
import org.bitcoinppl.cove_core.KeyTeleportInput
import org.bitcoinppl.cove_core.MultiFormat
import org.bitcoinppl.cove_core.StringOrData
import java.util.UUID

private const val SENSITIVE_CLIPBOARD_TOKEN = "org.bitcoinppl.cove.keyteleport.CLIP_TOKEN"

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
        ?: transferMessageForDisplay()
        ?: serviceMessageForDisplay()
        ?: "Something went wrong with KeyTeleport."

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

private fun KeyTeleportAlert.transferMessageForDisplay(): String? =
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
        else -> null
    }

private fun KeyTeleportAlert.serviceMessageForDisplay(): String? =
    when (this) {
        is KeyTeleportAlert.ImportFailed -> v1
        is KeyTeleportAlert.Keychain -> v1
        is KeyTeleportAlert.Protocol -> v1
        is KeyTeleportAlert.Database -> v1
        else -> null
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
        SensitiveClipboardExpiry.schedule(context.applicationContext, token)
        clipboard.setPrimaryClip(clip)
    } else {
        clipboard.setPrimaryClip(clip)
    }
}

/**
 * Clear a sensitive clip that outlived its deadline while the app was backgrounded.
 *
 * Background clipboard reads are denied on Android 10+, so an expiry that could not confirm
 * ownership defers to this foreground sweep.
 */
internal fun clearExpiredSensitiveClipboard(context: Context) {
    SensitiveClipboardExpiry.clearIfExpired(context.applicationContext)
}

private object SensitiveClipboardExpiry {
    private const val PREFERENCES_NAME = "keyteleport_sensitive_clipboard"
    private const val TOKEN_KEY = "token"
    private const val EXPIRES_AT_EPOCH_MILLIS_KEY = "expires_at_epoch_millis"
    private const val LOG_TAG = "SensitiveClipboard"

    private val handler = Handler(Looper.getMainLooper())
    private var pendingClear: Runnable? = null
    private var currentToken: String? = null
    private var deadlineElapsedRealtime = 0L
    private var retriesAfterDeadline = 0

    fun schedule(
        context: Context,
        token: String,
    ) {
        cancelPending()
        currentToken = token
        deadlineElapsedRealtime = SystemClock.elapsedRealtime() + SENSITIVE_CLIPBOARD_LIFETIME_MILLIS
        retriesAfterDeadline = 0

        val persisted =
            preferences(context)
                .edit()
                .putString(TOKEN_KEY, token)
                .putLong(
                    EXPIRES_AT_EPOCH_MILLIS_KEY,
                    System.currentTimeMillis() + SENSITIVE_CLIPBOARD_LIFETIME_MILLIS,
                )
                .commit()
        if (!persisted) {
            Log.w(LOG_TAG, "Unable to persist sensitive clipboard expiry")
        }

        postClear(context, SENSITIVE_CLIPBOARD_LIFETIME_MILLIS)
    }

    fun clearIfExpired(context: Context) {
        if (currentToken == null && !restore(context)) return
        if (SystemClock.elapsedRealtime() < deadlineElapsedRealtime) return

        retriesAfterDeadline = 0
        clearIfOwned(context)
    }

    private fun restore(context: Context): Boolean {
        val preferences = preferences(context)
        val restored =
            restoreSensitiveClipboardExpiry(
                token = preferences.getString(TOKEN_KEY, null),
                expiresAtEpochMillis = preferences.getLong(EXPIRES_AT_EPOCH_MILLIS_KEY, 0),
                nowEpochMillis = System.currentTimeMillis(),
            )
        if (restored == null) {
            preferences.edit().clear().apply()
            return false
        }

        currentToken = restored.token
        deadlineElapsedRealtime =
            SystemClock.elapsedRealtime() + restored.remainingLifetimeMillis
        retriesAfterDeadline = 0
        if (restored.remainingLifetimeMillis > 0) {
            postClear(context, restored.remainingLifetimeMillis)
        }

        return true
    }

    private fun clearIfOwned(context: Context) {
        val expectedToken = currentToken ?: return
        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        val description = runCatching { clipboard.primaryClipDescription }.getOrNull()
        val token = runCatching { description?.extras?.getString(SENSITIVE_CLIPBOARD_TOKEN) }.getOrNull()
        val remainingLifetimeMillis = deadlineElapsedRealtime - SystemClock.elapsedRealtime()

        val decision =
            resolveSensitiveClipboardClear(
                descriptionReadable = description != null,
                tokenMatches = token == expectedToken,
                remainingLifetimeMillis = remainingLifetimeMillis,
                retriesAfterDeadline = retriesAfterDeadline,
            )

        when (decision) {
            SensitiveClipboardClearDecision.ClearAndForget -> {
                runCatching(clipboard::clearPrimaryClip)
                forget(context)
            }

            SensitiveClipboardClearDecision.ForgetOnly -> forget(context)

            is SensitiveClipboardClearDecision.Retry -> {
                if (remainingLifetimeMillis <= 0) retriesAfterDeadline += 1
                postClear(context, decision.delayMillis)
            }

            // the token stays so the next foreground sweep can finish the clear
            SensitiveClipboardClearDecision.KeepForForeground -> cancelPending()
        }
    }

    private fun postClear(
        context: Context,
        delayMillis: Long,
    ) {
        cancelPending()
        pendingClear =
            Runnable {
                clearIfOwned(context)
            }.also {
                handler.postDelayed(it, delayMillis)
            }
    }

    private fun cancelPending() {
        pendingClear?.let(handler::removeCallbacks)
        pendingClear = null
    }

    private fun forget(context: Context) {
        cancelPending()
        currentToken = null
        retriesAfterDeadline = 0
        preferences(context).edit().clear().apply()
    }

    private fun preferences(context: Context) =
        context.getSharedPreferences(PREFERENCES_NAME, Context.MODE_PRIVATE)
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
