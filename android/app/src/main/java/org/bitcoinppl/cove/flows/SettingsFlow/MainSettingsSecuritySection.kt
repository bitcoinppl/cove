package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.biometric.BiometricManager
import androidx.biometric.BiometricPrompt
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Fingerprint
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.LockOpen
import androidx.compose.material.icons.filled.TheaterComedy
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.core.content.ContextCompat
import org.bitcoinppl.cove.Auth
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.findFragmentActivity
import org.bitcoinppl.cove.localizedMessage
import org.bitcoinppl.cove.views.MaterialDivider
import org.bitcoinppl.cove.views.MaterialSection
import org.bitcoinppl.cove.views.MaterialSettingsItem
import org.bitcoinppl.cove.views.NumberPadPinView
import org.bitcoinppl.cove.views.SectionHeader
import org.bitcoinppl.cove_core.AuthManagerAction
import org.bitcoinppl.cove_core.AuthManagerException
import org.bitcoinppl.cove_core.AuthType
import org.bitcoinppl.cove_core.PinUpdateFailure
import org.bitcoinppl.cove_core.SecurityAlertState
import org.bitcoinppl.cove_core.SecuritySettingsAction
import org.bitcoinppl.cove_core.SecuritySettingsResult
import org.bitcoinppl.cove_core.SecuritySheetState

@Composable
internal fun SecuritySection(app: org.bitcoinppl.cove.AppManager) {
    val context = LocalContext.current
    val activity = context.findFragmentActivity()
    val auth = Auth
    val biometricManager = remember { BiometricManager.from(context) }

    val isBiometricAvailable =
        remember {
            biometricManager.canAuthenticate(BiometricManager.Authenticators.BIOMETRIC_STRONG) ==
                BiometricManager.BIOMETRIC_SUCCESS
        }

    // sheet and alert state (using Rust enums)
    var sheetState: SecuritySheetState by remember { mutableStateOf(SecuritySheetState.NONE) }
    var alertState: SecurityAlertState? by remember { mutableStateOf(null) }

    // local state for decoy mode (settings changes only affect UI, not persisted)
    var decoyModePinEnabled by remember { mutableStateOf(true) }
    var decoyModeFaceIdEnabled by remember { mutableStateOf(false) }
    var decoyModeWipeDataPinEnabled by remember { mutableStateOf(false) }
    var decoyModeDecoyPinEnabled by remember { mutableStateOf(false) }

    // track which action triggered decoy mode update (for local state updates)
    var lastDecoyAction: SecuritySettingsAction? by remember { mutableStateOf(null) }

    // computed toggle values
    val isBiometricEnabled =
        if (auth.isInDecoyMode()) {
            decoyModeFaceIdEnabled
        } else {
            auth.type == AuthType.BOTH || auth.type == AuthType.BIOMETRIC
        }

    val isPinEnabled =
        if (auth.isInDecoyMode()) {
            decoyModePinEnabled
        } else {
            auth.type == AuthType.BOTH || auth.type == AuthType.PIN
        }

    val isWipeDataPinEnabled =
        if (auth.isInDecoyMode()) {
            decoyModeWipeDataPinEnabled
        } else {
            auth.isWipeDataPinEnabled
        }

    val isDecoyPinEnabled =
        if (auth.isInDecoyMode()) {
            decoyModeDecoyPinEnabled
        } else {
            auth.isDecoyPinEnabled
        }

    // handle security result from Rust validation
    fun handleSecurityResult(result: SecuritySettingsResult, action: SecuritySettingsAction) {
        when (result) {
            is SecuritySettingsResult.ProceedToSheet -> sheetState = result.v1
            is SecuritySettingsResult.ShowAlert -> alertState = result.v1
            is SecuritySettingsResult.DecoyModeLocalUpdate -> {
                lastDecoyAction = action
                when (action) {
                    is SecuritySettingsAction.ToggleBiometric -> decoyModeFaceIdEnabled = action.enable
                    is SecuritySettingsAction.TogglePin -> decoyModePinEnabled = action.enable
                    is SecuritySettingsAction.ToggleWipeDataPin -> decoyModeWipeDataPinEnabled = action.enable
                    is SecuritySettingsAction.ToggleDecoyPin -> decoyModeDecoyPinEnabled = action.enable
                    is SecuritySettingsAction.ChangePin -> {}
                }
            }
        }
    }

    // toggle handlers using Rust validation
    fun onBiometricToggle(enable: Boolean) {
        val action = SecuritySettingsAction.ToggleBiometric(enable)
        val result = auth.validateSecurityAction(action, app.unverifiedWalletIds())
        handleSecurityResult(result, action)
    }

    fun onPinToggle(enable: Boolean) {
        val action = SecuritySettingsAction.TogglePin(enable)
        val result = auth.validateSecurityAction(action, app.unverifiedWalletIds())
        handleSecurityResult(result, action)
    }

    fun onWipeDataPinToggle(enable: Boolean) {
        val action = SecuritySettingsAction.ToggleWipeDataPin(enable)
        val result = auth.validateSecurityAction(action, app.unverifiedWalletIds())
        handleSecurityResult(result, action)
    }

    fun onDecoyPinToggle(enable: Boolean) {
        val action = SecuritySettingsAction.ToggleDecoyPin(enable)
        val result = auth.validateSecurityAction(action, app.unverifiedWalletIds())
        handleSecurityResult(result, action)
    }

    // setter functions
    fun setPin(pin: String) {
        if (auth.isInDecoyMode()) {
            decoyModePinEnabled = true
            sheetState = SecuritySheetState.NONE
            return
        }
        auth.dispatch(AuthManagerAction.SetPin(pin))
        sheetState = SecuritySheetState.NONE
    }

    fun setWipeDataPin(pin: String) {
        sheetState = SecuritySheetState.NONE
        if (auth.isInDecoyMode()) {
            decoyModeWipeDataPinEnabled = true
            return
        }

        try {
            auth.setWipeDataPin(pin)
        } catch (e: AuthManagerException) {
            Log.e("SecuritySection", "failed to set wipe data PIN", e)
            alertState = SecurityAlertState.ExtraSetPinError(PinUpdateFailure.UPDATE_FAILED)
        }
    }

    fun setDecoyPin(pin: String) {
        sheetState = SecuritySheetState.NONE
        if (auth.isInDecoyMode()) {
            decoyModeDecoyPinEnabled = true
            return
        }

        try {
            auth.setDecoyPin(pin)
        } catch (e: AuthManagerException) {
            Log.e("SecuritySection", "failed to set decoy PIN", e)
            alertState = SecurityAlertState.ExtraSetPinError(PinUpdateFailure.UPDATE_FAILED)
        }
    }

    val actionCancel = stringResource(R.string.action_cancel)
    val enableBiometricTitle = stringResource(R.string.settings_biometric_enable_title)
    val enableBiometricSubtitle = stringResource(R.string.settings_biometric_enable_subtitle)
    val disableBiometricTitle = stringResource(R.string.settings_biometric_disable_title)
    val disableBiometricSubtitle = stringResource(R.string.settings_biometric_disable_subtitle)

    val enableBiometricPromptInfo =
        remember(enableBiometricTitle, enableBiometricSubtitle, actionCancel) {
            BiometricPrompt.PromptInfo
                .Builder()
                .setTitle(enableBiometricTitle)
                .setSubtitle(enableBiometricSubtitle)
                .setNegativeButtonText(actionCancel)
                .build()
        }

    val disableBiometricPromptInfo =
        remember(disableBiometricTitle, disableBiometricSubtitle, actionCancel) {
            BiometricPrompt.PromptInfo
                .Builder()
                .setTitle(disableBiometricTitle)
                .setSubtitle(disableBiometricSubtitle)
                .setNegativeButtonText(actionCancel)
                .build()
        }

    // create fresh BiometricPrompt for enabling biometric (avoids stale closure issues)
    fun triggerEnableBiometric() {
        val act = activity ?: return
        BiometricPrompt(
            act,
            ContextCompat.getMainExecutor(context),
            object : BiometricPrompt.AuthenticationCallback() {
                override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
                    super.onAuthenticationError(errorCode, errString)
                    sheetState = SecuritySheetState.NONE
                }

                override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                    super.onAuthenticationSucceeded(result)
                    auth.dispatch(AuthManagerAction.EnableBiometric)
                    sheetState = SecuritySheetState.NONE
                }

                override fun onAuthenticationFailed() {
                    super.onAuthenticationFailed()
                    Log.w("SecuritySection", "Biometric authentication failed - user can retry")
                }
            },
        ).authenticate(enableBiometricPromptInfo)
    }

    // create fresh BiometricPrompt for disabling biometric (when no PIN is set)
    fun triggerDisableBiometric() {
        val act = activity ?: return
        BiometricPrompt(
            act,
            ContextCompat.getMainExecutor(context),
            object : BiometricPrompt.AuthenticationCallback() {
                override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
                    super.onAuthenticationError(errorCode, errString)
                    sheetState = SecuritySheetState.NONE
                }

                override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                    super.onAuthenticationSucceeded(result)
                    auth.dispatch(AuthManagerAction.DisableBiometric)
                    sheetState = SecuritySheetState.NONE
                }

                override fun onAuthenticationFailed() {
                    super.onAuthenticationFailed()
                    Log.w("SecuritySection", "Biometric authentication failed - user can retry")
                }
            },
        ).authenticate(disableBiometricPromptInfo)
    }

    // trigger biometric prompt when entering biometric states
    LaunchedEffect(sheetState) {
        when (sheetState) {
            SecuritySheetState.ENABLE_BIOMETRIC -> {
                triggerEnableBiometric()
            }
            SecuritySheetState.DISABLE_BIOMETRIC -> {
                // only use biometric prompt if no PIN is set (biometric-only auth)
                if (auth.type == AuthType.BIOMETRIC) {
                    triggerDisableBiometric()
                }
            }
            else -> {}
        }
    }

    SectionHeader(stringResource(R.string.settings_title_security))
    MaterialSection {
        Column {
            var itemCount = 0

            // biometric toggle
            if (isBiometricAvailable) {
                MaterialSettingsItem(
                    title = stringResource(R.string.settings_security_enable_biometric_title),
                    icon = Icons.Default.Fingerprint,
                    isSwitch = true,
                    switchCheckedState = isBiometricEnabled,
                    onCheckChanged = { enabled -> onBiometricToggle(enabled) },
                )
                itemCount++
            }

            // PIN toggle
            if (itemCount > 0) MaterialDivider()
            MaterialSettingsItem(
                title = stringResource(R.string.settings_security_enable_pin_title),
                icon = Icons.Default.Lock,
                isSwitch = true,
                switchCheckedState = isPinEnabled,
                onCheckChanged = { enabled -> onPinToggle(enabled) },
            )
            itemCount++

            // show additional PIN options when PIN is enabled
            if (isPinEnabled) {
                // change PIN
                MaterialDivider()
                MaterialSettingsItem(
                    title = stringResource(R.string.settings_security_change_pin_title),
                    icon = Icons.Default.LockOpen,
                    onClick = { sheetState = SecuritySheetState.CHANGE_PIN },
                )
                itemCount++

                // wipe data PIN toggle
                MaterialDivider()
                MaterialSettingsItem(
                    title = stringResource(R.string.settings_security_enable_wipe_data_pin_title),
                    icon = Icons.Default.Warning,
                    isSwitch = true,
                    switchCheckedState = isWipeDataPinEnabled,
                    onCheckChanged = { enabled -> onWipeDataPinToggle(enabled) },
                )
                itemCount++

                // decoy PIN toggle
                MaterialDivider()
                MaterialSettingsItem(
                    title = stringResource(R.string.settings_security_decoy_enable_title),
                    icon = Icons.Default.TheaterComedy,
                    isSwitch = true,
                    switchCheckedState = isDecoyPinEnabled,
                    onCheckChanged = { enabled -> onDecoyPinToggle(enabled) },
                )
            }
        }
    }

    // alert dialogs
    alertState?.let { state ->
        SecurityAlertDialog(
            state = state,
            onDismiss = { alertState = null },
            onConfirmSheet = { nextSheet ->
                alertState = null
                sheetState = nextSheet
            },
            onConfirmAlert = { nextAlert ->
                alertState = nextAlert
            },
            auth = auth,
            app = app,
        )
    }

    // full-screen sheet dialogs
    // exclude ENABLE_BIOMETRIC (handled by biometric prompt)
    // exclude DISABLE_BIOMETRIC when auth type is BIOMETRIC only (also handled by biometric prompt)
    val showSheetDialog =
        sheetState != SecuritySheetState.NONE &&
            sheetState != SecuritySheetState.ENABLE_BIOMETRIC &&
            !(sheetState == SecuritySheetState.DISABLE_BIOMETRIC && auth.type == AuthType.BIOMETRIC)

    if (showSheetDialog) {
        SecuritySheetDialog(
            state = sheetState,
            onDismiss = { sheetState = SecuritySheetState.NONE },
            onNextState = { nextState -> sheetState = nextState },
            onSetPin = ::setPin,
            onSetWipeDataPin = ::setWipeDataPin,
            onSetDecoyPin = ::setDecoyPin,
            auth = auth,
            onAlertState = { alertState = it },
        )
    }
}

@Composable
private fun SecurityAlertDialog(
    state: SecurityAlertState,
    onDismiss: () -> Unit,
    onConfirmSheet: (SecuritySheetState) -> Unit,
    onConfirmAlert: (SecurityAlertState) -> Unit,
    auth: org.bitcoinppl.cove.AuthManager,
    app: org.bitcoinppl.cove.AppManager,
) {
    when (state) {
        is SecurityAlertState.UnverifiedWallets -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(stringResource(R.string.settings_security_unverified_wallets_title)) },
                text = {
                    Text(stringResource(R.string.settings_security_unverified_wallets_message))
                },
                confirmButton = {
                    TextButton(
                        onClick = {
                            try {
                                app.selectWalletOrThrow(state.walletId)
                            } catch (e: Exception) {
                                Log.e("SecuritySection", "Failed to select wallet ${state.walletId}", e)
                            }
                            onDismiss()
                        },
                    ) {
                        Text(stringResource(R.string.settings_action_go_to_wallet))
                    }
                },
                dismissButton = {
                    TextButton(onClick = onDismiss) {
                        Text(stringResource(R.string.action_cancel))
                    }
                },
            )
        }

        SecurityAlertState.ConfirmEnableWipeMePin -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(stringResource(R.string.common_remaining_are_you_sure)) },
                text = {
                    Text(stringResource(R.string.settings_security_wipe_data_confirm_message))
                },
                confirmButton = {
                    TextButton(onClick = { onConfirmSheet(SecuritySheetState.ENABLE_WIPE_DATA_PIN) }) {
                        Text(stringResource(R.string.settings_action_yes_enable_wipe_data_pin))
                    }
                },
                dismissButton = {
                    TextButton(onClick = onDismiss) {
                        Text(stringResource(R.string.action_cancel))
                    }
                },
            )
        }

        SecurityAlertState.ConfirmDecoyPin -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(stringResource(R.string.common_remaining_are_you_sure)) },
                text = {
                    Text(stringResource(R.string.settings_security_decoy_confirm_message))
                },
                confirmButton = {
                    TextButton(onClick = { onConfirmSheet(SecuritySheetState.ENABLE_DECOY_PIN) }) {
                        Text(stringResource(R.string.settings_action_yes_enable_decoy_pin))
                    }
                },
                dismissButton = {
                    TextButton(onClick = onDismiss) {
                        Text(stringResource(R.string.action_cancel))
                    }
                },
            )
        }

        SecurityAlertState.NotePinRequired -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(stringResource(R.string.settings_security_pin_required_title)) },
                text = { Text(stringResource(R.string.settings_security_pin_required_message)) },
                confirmButton = {
                    TextButton(onClick = onDismiss) {
                        Text(stringResource(R.string.action_ok))
                    }
                },
            )
        }

        SecurityAlertState.NoteFaceIdDisablingForWipeMePin -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(stringResource(R.string.settings_security_disable_biometric_unlock_title)) },
                text = {
                    Text(stringResource(R.string.settings_security_disable_biometric_unlock_message))
                },
                confirmButton = {
                    TextButton(
                        onClick = {
                            auth.dispatch(AuthManagerAction.DisableBiometric)
                            onConfirmAlert(SecurityAlertState.ConfirmEnableWipeMePin)
                        },
                    ) {
                        Text(stringResource(R.string.settings_action_disable_biometric))
                    }
                },
                dismissButton = {
                    TextButton(onClick = onDismiss) {
                        Text(stringResource(R.string.action_cancel))
                    }
                },
            )
        }

        SecurityAlertState.NoteFaceIdDisablingForDecoyPin -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(stringResource(R.string.settings_security_disable_biometric_unlock_title)) },
                text = {
                    Text(stringResource(R.string.settings_security_disable_biometric_unlock_message))
                },
                confirmButton = {
                    TextButton(
                        onClick = {
                            auth.dispatch(AuthManagerAction.DisableBiometric)
                            onConfirmAlert(SecurityAlertState.ConfirmDecoyPin)
                        },
                    ) {
                        Text(stringResource(R.string.settings_action_disable_biometric))
                    }
                },
                dismissButton = {
                    TextButton(onClick = onDismiss) {
                        Text(stringResource(R.string.action_cancel))
                    }
                },
            )
        }

        SecurityAlertState.NoteNoFaceIdWhenTrickPins -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(stringResource(R.string.settings_security_trick_pin_unavailable_title)) },
                text = {
                    Text(stringResource(R.string.settings_security_no_biometric_with_trick_pins_message))
                },
                confirmButton = {
                    TextButton(onClick = { onConfirmSheet(SecuritySheetState.REMOVE_ALL_TRICK_PINS) }) {
                        Text(stringResource(R.string.settings_action_yes_disable_trick_pins))
                    }
                },
                dismissButton = {
                    TextButton(onClick = onDismiss) {
                        Text(stringResource(R.string.action_cancel))
                    }
                },
            )
        }

        SecurityAlertState.NoteNoFaceIdWhenWipeMePin -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(stringResource(R.string.settings_security_trick_pin_unavailable_title)) },
                text = { Text(stringResource(R.string.settings_security_no_biometric_with_wipe_message)) },
                confirmButton = {
                    TextButton(
                        onClick = {
                            // if no decoy PIN, we can enable biometric after removing wipe data PIN
                            val nextSheet =
                                if (!auth.isDecoyPinEnabled) {
                                    SecuritySheetState.REMOVE_WIPE_DATA_PIN_THEN_ENABLE_BIOMETRIC
                                } else {
                                    SecuritySheetState.REMOVE_WIPE_DATA_PIN
                                }
                            onConfirmSheet(nextSheet)
                        },
                    ) {
                        Text(stringResource(R.string.settings_action_disable_wipe_data_pin))
                    }
                },
                dismissButton = {
                    TextButton(onClick = onDismiss) {
                        Text(stringResource(R.string.action_cancel))
                    }
                },
            )
        }

        SecurityAlertState.NoteNoFaceIdWhenDecoyPin -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(stringResource(R.string.settings_security_trick_pin_unavailable_title)) },
                text = { Text(stringResource(R.string.settings_security_no_biometric_with_decoy_message)) },
                confirmButton = {
                    TextButton(
                        onClick = {
                            // if no wipe data PIN, we can enable biometric after removing decoy PIN
                            val nextSheet =
                                if (!auth.isWipeDataPinEnabled) {
                                    SecuritySheetState.REMOVE_DECOY_PIN_THEN_ENABLE_BIOMETRIC
                                } else {
                                    SecuritySheetState.REMOVE_DECOY_PIN
                                }
                            onConfirmSheet(nextSheet)
                        },
                    ) {
                        Text(stringResource(R.string.settings_action_disable_decoy_pin))
                    }
                },
                dismissButton = {
                    TextButton(onClick = onDismiss) {
                        Text(stringResource(R.string.action_cancel))
                    }
                },
            )
        }

        is SecurityAlertState.ExtraSetPinError -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(stringResource(R.string.settings_security_generic_error_title)) },
                text = { Text(state.failure.localizedMessage().asString()) },
                confirmButton = {
                    TextButton(onClick = onDismiss) {
                        Text(stringResource(R.string.action_ok))
                    }
                },
            )
        }
    }
}

@Composable
private fun SecuritySheetDialog(
    state: SecuritySheetState,
    onDismiss: () -> Unit,
    onNextState: (SecuritySheetState) -> Unit,
    onSetPin: (String) -> Unit,
    onSetWipeDataPin: (String) -> Unit,
    onSetDecoyPin: (String) -> Unit,
    auth: org.bitcoinppl.cove.AuthManager,
    onAlertState: (SecurityAlertState) -> Unit,
) {
    val enterCurrentPinTitle = stringResource(R.string.settings_pin_enter_current)

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
            when (state) {
                SecuritySheetState.NEW_PIN -> {
                    NewPinView(
                        onComplete = onSetPin,
                        backAction = onDismiss,
                    )
                }

                SecuritySheetState.REMOVE_PIN -> {
                    NumberPadPinView(
                        title = enterCurrentPinTitle,
                        isPinCorrect = { pin ->
                            if (auth.isInDecoyMode()) auth.checkDecoyPin(pin) else auth.checkPin(pin)
                        },
                        backAction = onDismiss,
                        onUnlock = {
                            if (auth.isInDecoyMode()) {
                                onDismiss()
                                return@NumberPadPinView
                            }
                            auth.dispatch(AuthManagerAction.DisablePin)
                            auth.dispatch(AuthManagerAction.DisableWipeDataPin)
                            onDismiss()
                        },
                    )
                }

                SecuritySheetState.CHANGE_PIN -> {
                    ChangePinView(
                        isPinCorrect = { pin ->
                            if (auth.isInDecoyMode()) auth.checkDecoyPin(pin) else auth.checkPin(pin)
                        },
                        backAction = onDismiss,
                        onComplete = { pin ->
                            if (auth.isInDecoyMode()) {
                                onDismiss()
                                return@ChangePinView
                            }

                            // use Rust validation for new PIN
                            val error = auth.validateNewPin(pin)
                            if (error != null) {
                                onDismiss()
                                onAlertState(SecurityAlertState.ExtraSetPinError(error))
                                return@ChangePinView
                            }

                            onSetPin(pin)
                        },
                    )
                }

                SecuritySheetState.DISABLE_BIOMETRIC -> {
                    NumberPadPinView(
                        title = enterCurrentPinTitle,
                        isPinCorrect = { pin -> auth.checkPin(pin) },
                        backAction = onDismiss,
                        onUnlock = {
                            auth.dispatch(AuthManagerAction.DisableBiometric)
                            onDismiss()
                        },
                    )
                }

                SecuritySheetState.REMOVE_WIPE_DATA_PIN -> {
                    NumberPadPinView(
                        title = enterCurrentPinTitle,
                        isPinCorrect = { pin -> auth.checkPin(pin) },
                        backAction = onDismiss,
                        onUnlock = {
                            if (auth.isInDecoyMode()) {
                                onDismiss()
                                return@NumberPadPinView
                            }
                            auth.dispatch(AuthManagerAction.DisableWipeDataPin)
                            onDismiss()
                        },
                    )
                }

                SecuritySheetState.REMOVE_WIPE_DATA_PIN_THEN_ENABLE_BIOMETRIC -> {
                    NumberPadPinView(
                        title = enterCurrentPinTitle,
                        isPinCorrect = { pin -> auth.checkPin(pin) },
                        backAction = onDismiss,
                        onUnlock = {
                            auth.dispatch(AuthManagerAction.DisableWipeDataPin)
                            onNextState(SecuritySheetState.ENABLE_BIOMETRIC)
                        },
                    )
                }

                SecuritySheetState.REMOVE_DECOY_PIN -> {
                    NumberPadPinView(
                        title = enterCurrentPinTitle,
                        isPinCorrect = { pin -> auth.checkPin(pin) },
                        backAction = onDismiss,
                        onUnlock = {
                            auth.dispatch(AuthManagerAction.DisableDecoyPin)
                            onDismiss()
                        },
                    )
                }

                SecuritySheetState.REMOVE_DECOY_PIN_THEN_ENABLE_BIOMETRIC -> {
                    NumberPadPinView(
                        title = enterCurrentPinTitle,
                        isPinCorrect = { pin -> auth.checkPin(pin) },
                        backAction = onDismiss,
                        onUnlock = {
                            auth.dispatch(AuthManagerAction.DisableDecoyPin)
                            onNextState(SecuritySheetState.ENABLE_BIOMETRIC)
                        },
                    )
                }

                SecuritySheetState.REMOVE_ALL_TRICK_PINS -> {
                    NumberPadPinView(
                        title = enterCurrentPinTitle,
                        isPinCorrect = { pin -> auth.checkPin(pin) },
                        backAction = onDismiss,
                        onUnlock = {
                            auth.dispatch(AuthManagerAction.DisableDecoyPin)
                            auth.dispatch(AuthManagerAction.DisableWipeDataPin)
                            onNextState(SecuritySheetState.ENABLE_BIOMETRIC)
                        },
                    )
                }

                SecuritySheetState.ENABLE_WIPE_DATA_PIN -> {
                    WipeDataPinView(
                        onComplete = onSetWipeDataPin,
                        backAction = onDismiss,
                    )
                }

                SecuritySheetState.ENABLE_DECOY_PIN -> {
                    DecoyPinView(
                        onComplete = onSetDecoyPin,
                        backAction = onDismiss,
                    )
                }

                else -> {}
            }
        }
    }
}
