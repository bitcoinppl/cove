package org.bitcoinppl.cove.flows.TapSignerFlow

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
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
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove_core.TapSignerConfirmPinArgs
import org.bitcoinppl.cove_core.TapSignerNewPinArgs
import org.bitcoinppl.cove_core.TapSignerRoute

/**
 * new PIN creation screen
 * allows user to create a new 6-digit PIN
 */
@Composable
fun TapSignerNewPinView(
    app: AppManager,
    manager: TapSignerManager,
    args: TapSignerNewPinArgs,
    modifier: Modifier = Modifier,
) {
    var newPin by remember { mutableStateOf("") }

    // reset PIN when screen appears
    LaunchedEffect(Unit) {
        newPin = ""
    }

    // navigate to confirm PIN when 6 digits entered
    LaunchedEffect(newPin) {
        if (newPin.length == 6) {
            manager.navigate(
                TapSignerRoute.ConfirmPin(
                    TapSignerConfirmPinArgs(
                        tapSigner = args.tapSigner,
                        startingPin = args.startingPin,
                        newPin = newPin,
                        chainCode = args.chainCode,
                        action = args.action,
                    ),
                ),
            )
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
                text = "Create New PIN",
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

        // PIN circles
        Box(
            modifier = Modifier.fillMaxWidth(),
            contentAlignment = Alignment.Center,
        ) {
            PinCirclesView(pinLength = newPin.length)
        }

        // hidden text field
        HiddenPinTextField(
            value = newPin,
            onValueChange = { newPin = it },
        )

        Spacer(modifier = Modifier.height(40.dp))
    }
}
