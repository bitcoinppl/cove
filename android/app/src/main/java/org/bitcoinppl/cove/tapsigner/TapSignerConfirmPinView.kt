package org.bitcoinppl.cove.tapsigner

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.tween
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.AppAlertState
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove_core.TapSignerConfirmPinArgs
import org.bitcoinppl.cove_core.TapSignerPinAction
import org.bitcoinppl.cove_core.TapSignerRoute

/**
 * PIN confirmation screen
 * validates PIN match and triggers setup or change action
 */
@Composable
fun TapSignerConfirmPinView(
    app: AppManager,
    manager: TapSignerManager,
    args: TapSignerConfirmPinArgs,
    modifier: Modifier = Modifier,
) {
    var confirmPin by remember { mutableStateOf("") }
    val scope = rememberCoroutineScope()
    val offsetX = remember { Animatable(0f) }
    val context = LocalContext.current

    // reset PIN when screen appears
    LaunchedEffect(Unit) {
        confirmPin = ""
    }

    // check PIN when 6 digits entered
    LaunchedEffect(confirmPin) {
        if (confirmPin.length == 6) {
            delay(200)
            scope.launch {
                val activity = context as? android.app.Activity
                if (activity == null) {
                    app.alertState =
                        TaggedItem(
                            AppAlertState.General(
                                title = "Error",
                                message = "Unable to access NFC. Please try again.",
                            ),
                        )
                    confirmPin = ""
                    return@launch
                }

                checkPin(app, manager, args, confirmPin, offsetX, activity) {
                    confirmPin = ""
                }
            }
        }
    }

    Column(
        modifier =
            modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp),
        verticalArrangement = Arrangement.spacedBy(40.dp),
    ) {
        // back button
        Row(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(top = 20.dp),
            horizontalArrangement = Arrangement.Start,
        ) {
            TextButton(onClick = { manager.popRoute() }) {
                Icon(
                    imageVector = Icons.AutoMirrored.Filled.ArrowBack,
                    contentDescription = "Back",
                )
                Text("Back", fontWeight = FontWeight.SemiBold)
            }
        }

        // lock icon
        Icon(
            imageVector = Icons.Default.Lock,
            contentDescription = "Lock",
            modifier =
                Modifier
                    .size(100.dp)
                    .align(Alignment.CenterHorizontally),
            tint = MaterialTheme.colorScheme.primary,
        )

        // title and description
        Column(
            modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(20.dp),
        ) {
            Text(
                text = "Confirm New PIN",
                style = MaterialTheme.typography.headlineLarge,
                fontWeight = FontWeight.Bold,
            )

            Text(
                text =
                    "The PIN code is a security feature that prevents unauthorized access to your key. Please back it up and keep it safe. You'll need it for signing transactions.",
                style = MaterialTheme.typography.bodyMedium,
                textAlign = TextAlign.Center,
            )
        }

        // PIN circles with shake animation
        Box(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .offset { IntOffset(offsetX.value.toInt(), 0) },
            contentAlignment = Alignment.Center,
        ) {
            PinCirclesView(pinLength = confirmPin.length)
        }

        // hidden text field
        HiddenPinTextField(
            value = confirmPin,
            onValueChange = { confirmPin = it },
        )

        Spacer(modifier = Modifier.height(40.dp))
    }
}

private suspend fun checkPin(
    app: AppManager,
    manager: TapSignerManager,
    args: TapSignerConfirmPinArgs,
    confirmPin: String,
    offsetX: Animatable<Float, *>,
    activity: android.app.Activity,
    onPinMismatch: () -> Unit,
) {
    if (confirmPin != args.newPin) {
        // shake animation
        offsetX.animateTo(
            30f,
            animationSpec = tween(70),
        )
        offsetX.animateTo(-30f, animationSpec = tween(70))
        offsetX.animateTo(20f, animationSpec = tween(70))
        offsetX.animateTo(-20f, animationSpec = tween(70))
        offsetX.animateTo(10f, animationSpec = tween(70))
        offsetX.animateTo(-10f, animationSpec = tween(70))
        offsetX.animateTo(0f, animationSpec = tween(70))

        onPinMismatch()
        return
    }

    when (args.action) {
        TapSignerPinAction.SETUP -> setupTapSigner(app, manager, args, activity)
        TapSignerPinAction.CHANGE -> changeTapSignerPin(app, manager, args, activity)
    }
}

private suspend fun setupTapSigner(
    app: AppManager,
    manager: TapSignerManager,
    args: TapSignerConfirmPinArgs,
    activity: android.app.Activity,
) {
    val nfc = manager.getOrCreateNfc(args.tapSigner)

    // convert hex chain code to bytes if present
    val chainCodeBytes =
        args.chainCode?.let { hexCode ->
            try {
                hexCode.chunked(2).map { it.toInt(16).toByte() }.toByteArray()
            } catch (e: Exception) {
                null
            }
        }

    try {
        val response = nfc.setupTapSigner(args.startingPin, args.newPin, chainCodeBytes)

        when (response) {
            is org.bitcoinppl.cove_core.SetupCmdResponse.Complete -> {
                manager.resetRoute(TapSignerRoute.SetupSuccess(args.tapSigner, response.v1))
            }
            else -> {
                manager.resetRoute(TapSignerRoute.SetupRetry(args.tapSigner, response))
            }
        }
    } catch (e: Exception) {
        // check if we can continue from last response
        val lastResponse = nfc.lastResponse()
        val setupResponse =
            (lastResponse as? org.bitcoinppl.cove_core.TapSignerResponse.Setup)?.v1

        if (setupResponse != null) {
            manager.resetRoute(TapSignerRoute.SetupRetry(args.tapSigner, setupResponse))
        } else {
            // failed completely, go back to home
            android.util.Log.e("TapSignerConfirmPin", "Setup failed", e)
            app.sheetState = null
            app.alertState =
                TaggedItem(
                    AppAlertState.TapSignerSetupFailed(e.message ?: "Unknown error"),
                )
        }
    }
}

private suspend fun changeTapSignerPin(
    app: AppManager,
    manager: TapSignerManager,
    args: TapSignerConfirmPinArgs,
    activity: android.app.Activity,
) {
    val nfc = manager.getOrCreateNfc(args.tapSigner)

    try {
        nfc.changePin(args.startingPin, args.newPin)

        app.alertState =
            TaggedItem(
                AppAlertState.General(
                    title = "PIN Changed",
                    message = "Your TAPSIGNER PIN was changed successfully!",
                ),
            )
    } catch (e: Exception) {
        android.util.Log.e("TapSignerConfirmPin", "Error changing PIN", e)

        // check error type and show appropriate alert
        val errorMessage = e.message ?: "Unknown error"
        app.alertState =
            TaggedItem(
                AppAlertState.General(
                    title = "Error",
                    message = errorMessage,
                ),
            )
    }
}
