package org.bitcoinppl.cove

/**
 * sheet states that can be shown globally in the app
 * ported from iOS AppSheetState.swift
 */
sealed class AppSheetState {
    data object Qr : AppSheetState()
    data class TapSigner(val route: TapSignerRoute) : AppSheetState()
}
