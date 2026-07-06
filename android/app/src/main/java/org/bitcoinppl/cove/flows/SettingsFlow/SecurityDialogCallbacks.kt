@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.SettingsFlow

import org.bitcoinppl.cove_core.SecurityAlertState
import org.bitcoinppl.cove_core.SecuritySheetState

internal data class SecurityAlertDialogCallbacks(
    val onDismiss: () -> Unit,
    val onConfirmSheet: (SecuritySheetState) -> Unit,
    val onConfirmAlert: (SecurityAlertState) -> Unit,
)

internal class SecuritySheetPinCallbacks(
    val onSetPin: (String) -> Unit,
    val onSetWipeDataPin: (String) -> Unit,
    val onSetDecoyPin: (String) -> Unit,
)

internal class SecuritySheetDialogCallbacks(
    val onDismiss: () -> Unit,
    val onNextState: (SecuritySheetState) -> Unit,
    val pinCallbacks: SecuritySheetPinCallbacks,
    val onAlertState: (SecurityAlertState) -> Unit,
)
