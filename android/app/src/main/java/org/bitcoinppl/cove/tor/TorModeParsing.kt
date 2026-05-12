package org.bitcoinppl.cove.tor

import org.bitcoinppl.cove_core.TorMode

fun parseCoreTorMode(mode: String?): TorMode {
    return when (mode?.replace("_", "")?.lowercase()) {
        "orbot" -> TorMode.ORBOT
        "external" -> TorMode.EXTERNAL
        "builtin" -> TorMode.BUILT_IN
        else -> TorMode.BUILT_IN
    }
}
