package org.bitcoinppl.cove

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager

internal class ExternalKeyTeleportIntentHandler(
    private val context: Context,
) {
    private var handledSignature: String? = null

    fun handle(intent: Intent?) {
        val text = intent?.takeIf(::isAllowed)?.keyTeleportText() ?: return
        val signature = "${intent.action}:$text"
        val signatureMatches = signature == handledSignature
        if (shouldSkipHandledExternalIntent(
                signatureMatches = signatureMatches,
                hasActiveKeyTeleportFlow = AppManager.getInstance().keyTeleportManager != null,
            )
        ) {
            return
        }

        // the flow that consumed this link ended, so the same link may be opened again
        if (signatureMatches) {
            handledSignature = null
        }

        when (Scanner.handleKeyTeleportText(text)) {
            KeyTeleportIngestOutcome.INGESTED -> {
                handledSignature = signature
                intent.sanitize()
            }

            KeyTeleportIngestOutcome.REJECTED -> intent.sanitize()

            KeyTeleportIngestOutcome.NOT_KEY_TELEPORT -> Unit
        }
    }

    private fun isAllowed(intent: Intent): Boolean {
        if (intent.action != Intent.ACTION_VIEW && intent.action != Intent.ACTION_SEND) return false

        val alias = ComponentName(context.packageName, KEY_TELEPORT_LINK_ALIAS)
        val aliasEnabled =
            runCatching {
                when (context.packageManager.getComponentEnabledSetting(alias)) {
                    PackageManager.COMPONENT_ENABLED_STATE_ENABLED -> true
                    PackageManager.COMPONENT_ENABLED_STATE_DEFAULT ->
                        context.packageManager
                            .getActivityInfo(
                                alias,
                                PackageManager.ComponentInfoFlags.of(
                                    PackageManager.MATCH_DISABLED_COMPONENTS.toLong(),
                                ),
                            ).enabled
                    else -> false
                }
            }.getOrDefault(false)

        return shouldAcceptExternalKeyTeleportIntent(
            action = intent.action,
            componentClassName = intent.component?.className,
            keyTeleportAliasClassName = KEY_TELEPORT_LINK_ALIAS,
            keyTeleportAliasEnabled = aliasEnabled,
        )
    }
}

private fun Intent.keyTeleportText(): String? =
    when (action) {
        Intent.ACTION_VIEW -> dataString
        Intent.ACTION_SEND -> getStringExtra(Intent.EXTRA_TEXT)
        else -> null
    }?.trim()?.takeIf(String::isNotEmpty)

private fun Intent.sanitize() {
    action = Intent.ACTION_MAIN
    data = null
    type = null
    clipData = null
    removeExtra(Intent.EXTRA_TEXT)
}

/**
 * A repeated external link is only a duplicate delivery while the flow it opened is still alive;
 * once that flow ends the same link must be able to start a new transfer
 */
internal fun shouldSkipHandledExternalIntent(
    signatureMatches: Boolean,
    hasActiveKeyTeleportFlow: Boolean,
): Boolean = signatureMatches && hasActiveKeyTeleportFlow

internal fun shouldAcceptExternalKeyTeleportIntent(
    action: String?,
    componentClassName: String?,
    keyTeleportAliasClassName: String,
    keyTeleportAliasEnabled: Boolean,
): Boolean =
    when (action) {
        Intent.ACTION_SEND,
        Intent.ACTION_VIEW,
        ->
            keyTeleportAliasEnabled && componentClassName == keyTeleportAliasClassName

        else -> false
    }

private const val KEY_TELEPORT_LINK_ALIAS = "org.bitcoinppl.cove.KeyTeleportLinkActivity"
