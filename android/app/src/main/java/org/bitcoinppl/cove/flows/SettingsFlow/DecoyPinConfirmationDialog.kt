@file:Suppress("FunctionNaming", "PackageNaming")

package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.window.DialogProperties
import org.bitcoinppl.cove.AuthManager
import org.bitcoinppl.cove.views.lockaware.Dialog
import org.bitcoinppl.cove_core.SecuritySettingsAction

/**
 * Asks for the decoy PIN where main mode asks for its PIN, so decoy mode's Settings look the
 * same, then applies [action] to the toggles decoy mode shows
 */
@Composable
internal fun DecoyPinConfirmationDialog(
    action: SecuritySettingsAction,
    auth: AuthManager,
    onConfirmed: (SecuritySettingsAction) -> Unit,
    onDismiss: () -> Unit,
) {
    Dialog(
        onDismissRequest = onDismiss,
        properties = DialogProperties(usePlatformDefaultWidth = false),
    ) {
        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .background(Color.Black),
        ) {
            CurrentPinSheet(
                isPinCorrect = { pin -> auth.checkDecoyPin(pin) },
                backAction = onDismiss,
                onUnlock = { onConfirmed(action) },
            )
        }
    }
}
