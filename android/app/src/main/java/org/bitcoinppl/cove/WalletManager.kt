package org.bitcoinppl.cove

import androidx.compose.runtime.Stable

/**
 * wallet manager placeholder
 * full implementation will come in phase 2 of the plan
 * ported from iOS WalletManager.swift
 */
@Stable
class WalletManager(val id: WalletId) {
    private val tag = "WalletManager"

    internal val rust: RustWalletManager = RustWalletManager(id)

    init {
        android.util.Log.d(tag, "Initializing WalletManager for $id")
    }

    suspend fun forceWalletScan() {
        rust.forceWalletScan()
    }

    suspend fun updateWalletBalance() {
        // placeholder for now
        android.util.Log.d(tag, "updateWalletBalance called")
    }
}
