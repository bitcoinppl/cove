package org.bitcoinppl.cove.flows.TapSignerFlow

import androidx.compose.foundation.layout.Arrangement
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
import androidx.compose.material3.Button
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove_core.TapSignerConfirmPinArgs
import org.bitcoinppl.cove_core.TapSignerNewPinArgs
import org.bitcoinppl.cove_core.TapSignerRoute

/** Enter the new card CVC. */
@Composable
fun TapSignerNewPinView(
    app: AppManager,
    manager: TapSignerManager,
    args: TapSignerNewPinArgs,
    modifier: Modifier = Modifier,
) {
    var newCvc by remember { mutableStateOf("") }
    val validCvc = isValidCvc(newCvc)

    DisposableEffect(Unit) {
        onDispose { newCvc = "" }
    }

    Column(
        modifier =
            modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp),
        verticalArrangement = Arrangement.spacedBy(32.dp),
    ) {
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

        Icon(
            imageVector = Icons.Default.Lock,
            contentDescription = "Lock",
            modifier = Modifier.size(100.dp).align(Alignment.CenterHorizontally),
            tint = MaterialTheme.colorScheme.primary,
        )

        Column(
            modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(20.dp),
        ) {
            Text(
                text = "Create New CVC",
                style = MaterialTheme.typography.headlineLarge,
                fontWeight = FontWeight.Bold,
            )

            Text(
                text =
                    "Choose a CVC of 6–32 ASCII digits. " +
                        "Store it safely because you need it for card operations.",
                style = MaterialTheme.typography.bodyMedium,
                textAlign = TextAlign.Center,
            )

            TapSignerCvcInput(
                value = newCvc,
                onValueChange = { newCvc = it },
                label = "New CVC",
                testTag = "tapSignerNew.newCvc",
            )
        }

        Button(
            onClick = {
                manager.navigate(
                    TapSignerRoute.ConfirmPin(
                        TapSignerConfirmPinArgs(
                            tapSigner = args.tapSigner,
                            startingPin = args.startingPin,
                            newPin = newCvc,
                            chainCode = args.chainCode,
                            action = args.action,
                        ),
                    ),
                )
            },
            enabled = validCvc,
            modifier = Modifier.fillMaxWidth().testTag("tapSignerNew.continue"),
        ) {
            Text("Continue")
        }

        Spacer(modifier = Modifier.height(20.dp))
    }
}
