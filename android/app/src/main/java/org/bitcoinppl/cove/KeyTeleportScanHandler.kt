package org.bitcoinppl.cove

import org.bitcoinppl.cove.flows.keyteleport.KeyTeleportFlowDirection
import org.bitcoinppl.cove_core.AppAlertState
import org.bitcoinppl.cove_core.KeyTeleportInput
import org.bitcoinppl.cove_core.KeyTeleportReceiverPacket
import org.bitcoinppl.cove_core.KeyTeleportRoute
import org.bitcoinppl.cove_core.KeyTeleportSenderPacket
import org.bitcoinppl.cove_core.MultiFormat
import org.bitcoinppl.cove_core.MultiFormatException
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.RouteFactory
import org.bitcoinppl.cove_core.StringOrData

private sealed interface KeyTeleportTextParseResult {
    data class Parsed(
        val multiFormat: MultiFormat,
    ) : KeyTeleportTextParseResult

    data object UnsupportedPsbt : KeyTeleportTextParseResult

    data object InvalidPacket : KeyTeleportTextParseResult

    data object NotKeyTeleport : KeyTeleportTextParseResult
}

/** Outcome of routing text that may hold a KeyTeleport packet */
internal enum class KeyTeleportIngestOutcome {
    /** the packet was accepted by the KeyTeleport flow */
    INGESTED,

    /** the text was a KeyTeleport packet, but it could not be used and an alert was shown */
    REJECTED,

    /** the text was not a KeyTeleport packet, so another handler may claim it */
    NOT_KEY_TELEPORT,
}

internal class KeyTeleportScanHandler(
    private val app: AppManager,
) {
    fun handleText(input: String): KeyTeleportIngestOutcome {
        val text = input.trim()
        val parseResult =
            try {
                KeyTeleportTextParseResult.Parsed(
                    StringOrData.String(text).tryIntoMultiFormat(),
                )
            } catch (_: MultiFormatException.KeyTeleportPsbtNotSupported) {
                KeyTeleportTextParseResult.UnsupportedPsbt
            } catch (_: Exception) {
                val normalized = text.lowercase()
                val looksLikeKeyTeleport =
                    normalized.contains("keyteleport.com") ||
                        normalized.startsWith("b\$2r") ||
                        normalized.startsWith("b\$2s") ||
                        normalized.startsWith("b\$2p")

                if (looksLikeKeyTeleport) {
                    KeyTeleportTextParseResult.InvalidPacket
                } else {
                    KeyTeleportTextParseResult.NotKeyTeleport
                }
            }

        return when (parseResult) {
            is KeyTeleportTextParseResult.Parsed ->
                when (val multiFormat = parseResult.multiFormat) {
                    is MultiFormat.KeyTeleportReceiver ->
                        ingestOutcome(handleReceiver(multiFormat.v1))

                    is MultiFormat.KeyTeleportSender ->
                        ingestOutcome(handleSender(multiFormat.v1))

                    else -> {
                        multiFormat.destroy()
                        KeyTeleportIngestOutcome.NOT_KEY_TELEPORT
                    }
                }

            KeyTeleportTextParseResult.UnsupportedPsbt -> {
                app.alertState =
                    TaggedItem(
                        AppAlertState.InvalidFileFormat(
                            "KeyTeleport PSBT packets are not supported yet.",
                        ),
                    )
                KeyTeleportIngestOutcome.REJECTED
            }

            KeyTeleportTextParseResult.InvalidPacket -> {
                app.alertState =
                    TaggedItem(
                        AppAlertState.InvalidFileFormat(
                            "This KeyTeleport packet could not be read.",
                        ),
                    )
                KeyTeleportIngestOutcome.REJECTED
            }

            KeyTeleportTextParseResult.NotKeyTeleport -> KeyTeleportIngestOutcome.NOT_KEY_TELEPORT
        }
    }

    fun handleReceiver(packet: KeyTeleportReceiverPacket): Boolean =
        ingestKeyTeleportPacket(
            input = KeyTeleportInput.Receiver(packet),
            direction = KeyTeleportFlowDirection.SEND,
            route = RouteFactory().keyTeleportSend(),
        )

    fun handleSender(packet: KeyTeleportSenderPacket): Boolean =
        ingestKeyTeleportPacket(
            input = KeyTeleportInput.Sender(packet),
            direction = KeyTeleportFlowDirection.RECEIVE,
            route = RouteFactory().keyTeleportReceive(),
        )

    private fun ingestKeyTeleportPacket(
        input: KeyTeleportInput,
        direction: KeyTeleportFlowDirection,
        route: Route,
    ): Boolean {
        val manager = app.getKeyTeleportManager()
        val ingested =
            if (canRouteKeyTeleportPacket(manager.activeFlowDirection, route)) {
                manager.ingest(input, direction)
            } else {
                input.destroy()
                false
            }

        if (!ingested) {
            showKeyTeleportFlowConflict()
            return false
        }

        if (app.currentRoute != route) {
            app.pushRoute(route)
        }

        return true
    }

    private fun canRouteKeyTeleportPacket(
        managerDirection: KeyTeleportFlowDirection?,
        targetRoute: Route,
    ): Boolean {
        val targetDirection = targetRoute.keyTeleportFlowDirection() ?: return false
        val activeRouteDirections =
            (listOf(app.router.default) + app.router.routes)
                .filterIsInstance<Route.KeyTeleport>()
                .mapNotNull(Route::keyTeleportFlowDirection)

        return canRouteKeyTeleportPacket(
            managerDirection = managerDirection,
            targetDirection = targetDirection,
            activeRouteDirections = activeRouteDirections,
            currentRouteDirection = app.currentRoute.keyTeleportFlowDirection(),
        )
    }

    private fun showKeyTeleportFlowConflict() {
        app.alertState =
            TaggedItem(
                AppAlertState.General(
                    title = "KeyTeleport",
                    message = "Finish the active KeyTeleport flow before opening a different transfer.",
                ),
            )
    }
}

private fun ingestOutcome(ingested: Boolean): KeyTeleportIngestOutcome =
    if (ingested) KeyTeleportIngestOutcome.INGESTED else KeyTeleportIngestOutcome.REJECTED

private fun Route.keyTeleportFlowDirection(): KeyTeleportFlowDirection? =
    when (this) {
        is Route.KeyTeleport ->
            when (v1) {
                KeyTeleportRoute.RECEIVE -> KeyTeleportFlowDirection.RECEIVE
                KeyTeleportRoute.SEND -> KeyTeleportFlowDirection.SEND
            }
        else -> null
    }

internal fun canRouteKeyTeleportPacket(
    managerDirection: KeyTeleportFlowDirection?,
    targetDirection: KeyTeleportFlowDirection,
    activeRouteDirections: List<KeyTeleportFlowDirection>,
    currentRouteDirection: KeyTeleportFlowDirection?,
): Boolean {
    val managerCanRoute = managerDirection == null || managerDirection == targetDirection
    val activeRoutesMatch = activeRouteDirections.all { it == targetDirection }
    val currentRouteMatches =
        activeRouteDirections.isEmpty() || currentRouteDirection == targetDirection

    return managerCanRoute && activeRoutesMatch && currentRouteMatches
}
