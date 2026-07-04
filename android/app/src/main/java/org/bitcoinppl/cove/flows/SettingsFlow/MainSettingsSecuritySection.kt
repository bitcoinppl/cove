package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.biometric.BiometricManager
import androidx.biometric.BiometricPrompt
import androidx.compose.foundation.layout.Column
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Fingerprint
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.LockOpen
import androidx.compose.material.icons.filled.TheaterComedy
import androidx.compose.material.icons.filled.Warning
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import androidx.core.content.ContextCompat
import org.bitcoinppl.cove.Auth
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove.findFragmentActivity
import org.bitcoinppl.cove.views.MaterialDivider
import org.bitcoinppl.cove.views.MaterialSection
import org.bitcoinppl.cove.views.MaterialSettingsItem
import org.bitcoinppl.cove.views.SectionHeader
import org.bitcoinppl.cove_core.AuthManagerAction
import org.bitcoinppl.cove_core.AuthManagerException
import org.bitcoinppl.cove_core.AuthType
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
    val decoyModeState = remember { SecurityDecoyModeState() }

    // computed toggle values
    val isBiometricEnabled = decoyModeState.isBiometricEnabled(auth)
    val isPinEnabled = decoyModeState.isPinEnabled(auth)
    val isWipeDataPinEnabled = decoyModeState.isWipeDataPinEnabled(auth)
    val isDecoyPinEnabled = decoyModeState.isDecoyPinEnabled(auth)

    // handle security result from Rust validation
    fun handleSecurityResult(result: SecuritySettingsResult, action: SecuritySettingsAction) {
        when (result) {
            is SecuritySettingsResult.ProceedToSheet -> sheetState = result.v1
            is SecuritySettingsResult.ShowAlert -> alertState = result.v1
            is SecuritySettingsResult.DecoyModeLocalUpdate -> decoyModeState.apply(action)
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
            decoyModeState.enablePin()
            sheetState = SecuritySheetState.NONE
            return
        }
        auth.dispatch(AuthManagerAction.SetPin(pin))
        sheetState = SecuritySheetState.NONE
    }

    fun setWipeDataPin(pin: String) {
        sheetState = SecuritySheetState.NONE
        if (auth.isInDecoyMode()) {
            decoyModeState.enableWipeDataPin()
            return
        }

        try {
            auth.setWipeDataPin(pin)
        } catch (e: AuthManagerException) {
            alertState = SecurityAlertState.ExtraSetPinError(e.message ?: "Unknown error")
        }
    }

    fun setDecoyPin(pin: String) {
        sheetState = SecuritySheetState.NONE
        if (auth.isInDecoyMode()) {
            decoyModeState.enableDecoyPin()
            return
        }

        try {
            auth.setDecoyPin(pin)
        } catch (e: AuthManagerException) {
            alertState = SecurityAlertState.ExtraSetPinError(e.message ?: "Unknown error")
        }
    }

    val enableBiometricPromptInfo =
        remember {
            BiometricPrompt.PromptInfo
                .Builder()
                .setTitle("Enable Biometric")
                .setSubtitle("Authenticate to enable biometric unlock")
                .setNegativeButtonText("Cancel")
                .build()
        }

    val disableBiometricPromptInfo =
        remember {
            BiometricPrompt.PromptInfo
                .Builder()
                .setTitle("Disable Biometric")
                .setSubtitle("Authenticate to disable biometric unlock")
                .setNegativeButtonText("Cancel")
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

    SectionHeader("Security")
    MaterialSection {
        Column {
            var itemCount = 0

            // biometric toggle
            if (isBiometricAvailable) {
                MaterialSettingsItem(
                    title = "Enable Biometric",
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
                title = "Enable PIN",
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
                    title = "Change PIN",
                    icon = Icons.Default.LockOpen,
                    onClick = { sheetState = SecuritySheetState.CHANGE_PIN },
                )
                itemCount++

                // wipe data PIN toggle
                MaterialDivider()
                MaterialSettingsItem(
                    title = "Enable Wipe Data PIN",
                    icon = Icons.Default.Warning,
                    isSwitch = true,
                    switchCheckedState = isWipeDataPinEnabled,
                    onCheckChanged = { enabled -> onWipeDataPinToggle(enabled) },
                )
                itemCount++

                // decoy PIN toggle
                MaterialDivider()
                MaterialSettingsItem(
                    title = "Enable Decoy PIN",
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
            callbacks =
                SecurityAlertDialogCallbacks(
                    onDismiss = { alertState = null },
                    onConfirmSheet = { nextSheet ->
                        alertState = null
                        sheetState = nextSheet
                    },
                    onConfirmAlert = { nextAlert ->
                        alertState = nextAlert
                    },
                ),
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
            callbacks =
                SecuritySheetDialogCallbacks(
                    onDismiss = { sheetState = SecuritySheetState.NONE },
                    onNextState = { nextState -> sheetState = nextState },
                    pinCallbacks =
                        SecuritySheetPinCallbacks(
                            onSetPin = ::setPin,
                            onSetWipeDataPin = ::setWipeDataPin,
                            onSetDecoyPin = ::setDecoyPin,
                        ),
                    onAlertState = { alertState = it },
                ),
            auth = auth,
        )
    }
}
