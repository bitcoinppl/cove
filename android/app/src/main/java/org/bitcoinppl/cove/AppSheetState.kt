package org.bitcoinppl.cove

import org.bitcoinppl.cove_core.*

/**
 * represents different sheet states that can be presented in the app
 * ported from iOS AppSheetState.swift
 */
sealed class AppSheetState {
    data object Qr : AppSheetState()

    data object Nfc : AppSheetState()

    data class TapSigner(
        val route: TapSignerRoute,
    ) : AppSheetState()
}
