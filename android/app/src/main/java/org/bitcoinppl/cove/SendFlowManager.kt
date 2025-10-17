package org.bitcoinppl.cove

import androidx.compose.runtime.Stable

/**
 * send flow manager placeholder
 * full implementation will come in phase 2 of the plan
 * ported from iOS SendFlowManager.swift
 */
@Stable
class SendFlowManager(
    internal val rust: RustSendFlowManager,
    var presenter: SendFlowPresenter
) {
    private val tag = "SendFlowManager"

    val id: WalletId
        get() = rust.id()

    init {
        android.util.Log.d(tag, "Initializing SendFlowManager for $id")
    }
}

/**
 * send flow presenter placeholder
 * handles presentation logic for send flow
 */
interface SendFlowPresenter {
    // placeholder for now, will be filled in phase 2
}
