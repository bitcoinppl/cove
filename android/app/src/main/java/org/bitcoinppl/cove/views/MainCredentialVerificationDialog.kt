package org.bitcoinppl.cove.views

import androidx.biometric.BiometricManager
import androidx.biometric.BiometricPrompt
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.DialogProperties
import androidx.core.content.ContextCompat
import org.bitcoinppl.cove.Auth
import org.bitcoinppl.cove.BiometricPolicy
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove.findFragmentActivity
import org.bitcoinppl.cove.views.lockaware.Dialog
import org.bitcoinppl.cove_core.AuthType
import java.util.concurrent.atomic.AtomicBoolean

private enum class CredentialScreen {
    BIOMETRIC,
    PIN,
}

/**
 * Asks for the main credential before a secret export, which only the app PIN or biometrics
 * satisfy, as iOS's MainCredentialVerificationView does
 *
 * It never locks the app, so the decoy and wipe data PINs are just wrong PINs here: neither
 * switches to the decoy wallets nor wipes. Cancel closes it without verifying
 */
@Composable
fun MainCredentialVerificationDialog(
    lockType: AuthType,
    onVerified: () -> Unit,
    onCancel: () -> Unit,
) {
    val auth = Auth
    val context = LocalContext.current
    val activity = context.findFragmentActivity()
    val biometricManager = remember { BiometricManager.from(context) }
    val usesBiometric =
        remember(lockType) {
            (lockType == AuthType.BIOMETRIC || lockType == AuthType.BOTH) &&
                BiometricPolicy.isAvailable(biometricManager)
        }

    var screen by remember(usesBiometric) {
        mutableStateOf(if (usesBiometric) CredentialScreen.BIOMETRIC else CredentialScreen.PIN)
    }
    var isPrompting by remember { mutableStateOf(false) }

    // the prompt can deliver a result after the caller took this dialog down, such as when a
    // pause cancelled the request, and that result must not complete the cancelled export
    val isShown = remember { AtomicBoolean(true) }
    var biometricPrompt by remember { mutableStateOf<BiometricPrompt?>(null) }
    DisposableEffect(Unit) {
        onDispose {
            isShown.set(false)
            biometricPrompt?.cancelAuthentication()
        }
    }

    val promptInfo =
        remember {
            BiometricPolicy.promptInfo(
                title = "Verify Identity",
                subtitle = "Confirm access to your private key",
                negativeButtonText = "Cancel",
            )
        }

    fun promptBiometric() {
        val act = activity ?: return
        if (isPrompting) return
        isPrompting = true

        val prompt =
            BiometricPrompt(
                act,
                ContextCompat.getMainExecutor(context),
                object : BiometricPrompt.AuthenticationCallback() {
                    override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
                        super.onAuthenticationError(errorCode, errString)
                        isPrompting = false
                        Log.i("MainCredentialVerification", "biometric prompt ended: code $errorCode")
                    }

                    override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                        super.onAuthenticationSucceeded(result)
                        isPrompting = false
                        if (!isShown.get()) return
                        onVerified()
                    }

                    override fun onAuthenticationFailed() {
                        super.onAuthenticationFailed()
                        Log.w("MainCredentialVerification", "biometric authentication failed, user can retry")
                    }
                },
            )

        biometricPrompt = prompt
        prompt.authenticate(promptInfo)
    }

    Dialog(
        onDismissRequest = onCancel,
        properties = DialogProperties(usePlatformDefaultWidth = false),
    ) {
        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .background(Color.Black)
                    .semantics { testTagsAsResourceId = true },
            contentAlignment = Alignment.Center,
        ) {
            val cancel =
                @Composable {
                    Text(
                        text = "Cancel",
                        color = Color.White,
                        fontSize = 17.sp,
                        modifier =
                            Modifier
                                .align(Alignment.TopEnd)
                                .clickable(onClick = onCancel)
                                .padding(24.dp),
                    )
                }

            when {
                // a biometric-only lock has no PIN to fall back to, as on the lock screen and iOS
                lockType == AuthType.BIOMETRIC && !usesBiometric -> {
                    PermissionsNeeded()
                    cancel()
                }

                screen == CredentialScreen.BIOMETRIC -> {
                    BiometricView(
                        showBoth = lockType == AuthType.BOTH,
                        onBiometricTap = ::promptBiometric,
                        onEnterPinTap = { screen = CredentialScreen.PIN },
                        onResetBiometricFlag = { isPrompting = false },
                    )
                    cancel()
                }

                // only the app PIN opens this, never a trick PIN
                else ->
                    NumberPadPinView(
                        title = "Enter Cove PIN",
                        isPinCorrect = { pin -> auth.checkPin(pin) },
                        backAction = onCancel,
                        onUnlock = { onVerified() },
                    )
            }
        }
    }
}
