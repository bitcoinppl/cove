@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import org.bitcoinppl.cove.AuthManager
import org.bitcoinppl.cove_core.AuthType
import org.bitcoinppl.cove_core.SecuritySettingsAction

internal class SecurityDecoyModeState {
    var pinEnabled by mutableStateOf(true)
        private set

    var faceIdEnabled by mutableStateOf(false)
        private set

    var wipeDataPinEnabled by mutableStateOf(false)
        private set

    var decoyPinEnabled by mutableStateOf(false)
        private set

    var lastAction: SecuritySettingsAction? by mutableStateOf(null)
        private set

    fun isBiometricEnabled(auth: AuthManager): Boolean =
        if (auth.isInDecoyMode()) {
            faceIdEnabled
        } else {
            auth.type == AuthType.BOTH || auth.type == AuthType.BIOMETRIC
        }

    fun isPinEnabled(auth: AuthManager): Boolean =
        if (auth.isInDecoyMode()) {
            pinEnabled
        } else {
            auth.type == AuthType.BOTH || auth.type == AuthType.PIN
        }

    fun isWipeDataPinEnabled(auth: AuthManager): Boolean =
        if (auth.isInDecoyMode()) {
            wipeDataPinEnabled
        } else {
            auth.isWipeDataPinEnabled
        }

    fun isDecoyPinEnabled(auth: AuthManager): Boolean =
        if (auth.isInDecoyMode()) {
            decoyPinEnabled
        } else {
            auth.isDecoyPinEnabled
        }

    fun apply(action: SecuritySettingsAction) {
        lastAction = action

        when (action) {
            is SecuritySettingsAction.ToggleBiometric -> faceIdEnabled = action.enable
            is SecuritySettingsAction.TogglePin -> pinEnabled = action.enable
            is SecuritySettingsAction.ToggleWipeDataPin -> wipeDataPinEnabled = action.enable
            is SecuritySettingsAction.ToggleDecoyPin -> decoyPinEnabled = action.enable
            is SecuritySettingsAction.ChangePin -> {}
        }
    }

    fun enablePin() {
        pinEnabled = true
    }

    fun enableWipeDataPin() {
        wipeDataPinEnabled = true
    }

    fun enableDecoyPin() {
        decoyPinEnabled = true
    }
}
