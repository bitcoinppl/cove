package org.bitcoinppl.cove.views

import android.content.Intent
import android.provider.Settings
import androidx.biometric.BiometricManager
import androidx.biometric.BiometricPrompt
import androidx.biometric.BiometricPrompt.PromptInfo
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Fingerprint
import androidx.compose.material3.Button
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import org.bitcoinppl.cove.Auth
import org.bitcoinppl.cove.UnlockMode
import org.bitcoinppl.cove_core.AuthType

private enum class Screen {
    BIOMETRIC,
    PIN,
}

@Composable
fun LockView(
    content: @Composable () -> Unit,
) {
    val auth = Auth
    var screen by remember { mutableStateOf(Screen.BIOMETRIC) }
    val context = LocalContext.current
    val activity = context as? FragmentActivity
    val biometricManager = remember { BiometricManager.from(context) }

    val isBiometricAvailable =
        remember {
            biometricManager.canAuthenticate(BiometricManager.Authenticators.BIOMETRIC_STRONG) == BiometricManager.BIOMETRIC_SUCCESS
        }

    // keep promptInfo cached (it's just config, doesn't go stale)
    val promptInfo =
        remember {
            PromptInfo
                .Builder()
                .setTitle("Unlock Cove")
                .setSubtitle("Use your biometric to unlock")
                .setNegativeButtonText("Cancel")
                .build()
        }

    // trigger function creates FRESH BiometricPrompt each time (like iOS creates fresh LAContext)
    fun triggerBiometric() {
        val act = activity ?: return
        if (auth.isUsingBiometrics) return

        auth.isUsingBiometrics = true

        val biometricPrompt =
            BiometricPrompt(
                act,
                ContextCompat.getMainExecutor(context),
                object : BiometricPrompt.AuthenticationCallback() {
                    override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
                        super.onAuthenticationError(errorCode, errString)
                        auth.isUsingBiometrics = false
                    }

                    override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                        super.onAuthenticationSucceeded(result)
                        auth.isUsingBiometrics = false

                        // if in decoy mode, switch back to main mode (biometric = trusted user = main mode)
                        if (auth.isInDecoyMode()) {
                            auth.switchToMainMode()
                        }

                        auth.unlock()
                    }

                    override fun onAuthenticationFailed() {
                        super.onAuthenticationFailed()
                        // user can retry, don't clear flag yet
                    }
                },
            )

        biometricPrompt.authenticate(promptInfo)
    }

    // set screen to biometric when locked with biometric auth
    // (don't trigger biometric here - BiometricView's LaunchedEffect does that)
    LaunchedEffect(auth.isLocked, auth.type) {
        if (auth.isLocked && (auth.type == AuthType.BIOMETRIC || auth.type == AuthType.BOTH)) {
            if (isBiometricAvailable) {
                screen = Screen.BIOMETRIC
            }
        }
    }

    Box(modifier = Modifier.fillMaxSize()) {
        // main content
        content()

        // lock overlay
        AnimatedVisibility(
            visible = auth.isLocked,
            enter = slideInVertically { it },
            exit = slideOutVertically { it },
        ) {
            Box(
                modifier =
                    Modifier
                        .fillMaxSize()
                        .background(Color.Black),
                contentAlignment = Alignment.Center,
            ) {
                when {
                    // biometric not available but auth type is biometric only
                    auth.type == AuthType.BIOMETRIC && !isBiometricAvailable -> {
                        PermissionsNeeded()
                    }
                    // show biometric screen
                    screen == Screen.BIOMETRIC && (auth.type == AuthType.BIOMETRIC || auth.type == AuthType.BOTH) && isBiometricAvailable -> {
                        BiometricView(
                            showBoth = auth.type == AuthType.BOTH,
                            onBiometricTap = { triggerBiometric() },
                            onEnterPinTap = { screen = Screen.PIN },
                        )
                    }
                    // show PIN screen
                    else -> {
                        NumberPadPinView(
                            showPin = false,
                            isPinCorrect = { pin ->
                                when (auth.handleAndReturnUnlockMode(pin)) {
                                    UnlockMode.MAIN, UnlockMode.DECOY, UnlockMode.WIPE -> true
                                    UnlockMode.LOCKED -> false
                                }
                            },
                            onUnlock = { auth.unlock() },
                            backAction =
                                if (auth.type == AuthType.BOTH && isBiometricAvailable) {
                                    { screen = Screen.BIOMETRIC }
                                } else {
                                    null
                                },
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun PermissionsNeeded() {
    val context = LocalContext.current

    Column(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 50.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(
            text = "Cove needs permissions to use biometric authentication to unlock your wallet. Please open settings and enable biometric authentication.",
            style = MaterialTheme.typography.bodyMedium,
            textAlign = TextAlign.Center,
            color = Color.White,
        )

        Spacer(modifier = Modifier.height(20.dp))

        Button(
            onClick = {
                val intent = Intent(Settings.ACTION_SECURITY_SETTINGS)
                context.startActivity(intent)
            },
        ) {
            Text("Open Settings")
        }
    }
}

@Composable
private fun BiometricView(
    showBoth: Boolean,
    onBiometricTap: () -> Unit,
    onEnterPinTap: () -> Unit,
) {
    val lifecycleOwner = LocalLifecycleOwner.current

    // trigger biometric when Activity is RESUMED
    // BiometricPrompt fails silently if called during onStart, before onResume
    DisposableEffect(lifecycleOwner) {
        val observer =
            LifecycleEventObserver { _, event ->
                if (event == Lifecycle.Event.ON_RESUME) {
                    onBiometricTap()
                }
            }
        lifecycleOwner.lifecycle.addObserver(observer)

        // also trigger immediately if already resumed (fixes race condition where
        // composable enters composition after ON_RESUME event already fired)
        if (lifecycleOwner.lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED)) {
            onBiometricTap()
        }

        onDispose {
            lifecycleOwner.lifecycle.removeObserver(observer)
        }
    }

    Column(
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        // biometric icon and text
        Surface(
            modifier =
                Modifier
                    .size(100.dp)
                    .clickable(
                        onClick = onBiometricTap,
                        indication = null,
                        interactionSource = remember { MutableInteractionSource() },
                    ),
            shape = RoundedCornerShape(10.dp),
            color = Color.White.copy(alpha = 0.1f),
        ) {
            Column(
                modifier = Modifier.fillMaxSize(),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = androidx.compose.foundation.layout.Arrangement.Center,
            ) {
                Icon(
                    imageVector = Icons.Default.Fingerprint,
                    contentDescription = "Biometric",
                    tint = Color.White,
                    modifier = Modifier.size(48.dp),
                )
                Spacer(modifier = Modifier.height(6.dp))
                Text(
                    text = "Tap to Unlock",
                    fontSize = 10.sp,
                    color = Color.Gray,
                )
            }
        }

        // enter PIN button (only shown if auth type is BOTH)
        if (showBoth) {
            Spacer(modifier = Modifier.height(12.dp))

            Surface(
                modifier =
                    Modifier
                        .clickable(
                            onClick = onEnterPinTap,
                            indication = null,
                            interactionSource = remember { MutableInteractionSource() },
                        ),
                shape = RoundedCornerShape(10.dp),
                color = Color.White.copy(alpha = 0.1f),
            ) {
                Text(
                    text = "Enter Pin",
                    color = Color.White,
                    modifier = Modifier.padding(horizontal = 20.dp, vertical = 10.dp),
                    fontWeight = FontWeight.Normal,
                )
            }
        }
    }
}
