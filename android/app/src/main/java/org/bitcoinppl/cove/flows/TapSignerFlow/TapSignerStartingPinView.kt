package org.bitcoinppl.cove.flows.TapSignerFlow

import androidx.compose.foundation.background
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
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Icon
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
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.RectangleShape
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import coil3.compose.AsyncImage
import coil3.request.ImageRequest
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove_core.TapSignerNewPinArgs
import org.bitcoinppl.cove_core.TapSignerPinAction
import org.bitcoinppl.cove_core.TapSignerRoute

/** Enter the factory CVC as hexadecimal text before setup. */
@Composable
fun TapSignerStartingPinView(
    app: AppManager,
    manager: TapSignerManager,
    tapSigner: org.bitcoinppl.cove_core.tapcard.TapSigner,
    chainCode: String?,
    modifier: Modifier = Modifier,
) {
    var factoryCvc by remember { mutableStateOf("") }
    val validCvc = isValidCvcHex(factoryCvc)

    DisposableEffect(Unit) {
        onDispose { factoryCvc = "" }
    }

    Column(
        modifier =
            modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState()),
    ) {
        Box(
            modifier = Modifier.fillMaxWidth().background(Color(0xFF3A4254)),
        ) {
            Column {
                Row(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .padding(top = 20.dp, start = 10.dp, end = 10.dp),
                    horizontalArrangement = Arrangement.Start,
                ) {
                    TextButton(onClick = { manager.popRoute() }) {
                        Icon(
                            imageVector = Icons.AutoMirrored.Filled.ArrowBack,
                            contentDescription = "Back",
                            tint = Color.White,
                        )
                        Text("Back", fontWeight = FontWeight.SemiBold, color = Color.White)
                    }
                }

                AsyncImage(
                    model =
                        ImageRequest
                            .Builder(LocalContext.current)
                            .data("file:///android_asset/tapsigner_card.svg")
                            .build(),
                    contentDescription = "TapSigner Card",
                    modifier = Modifier.offset(y = 10.dp).clip(RectangleShape),
                )
            }
        }

        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(20.dp),
        ) {
            Spacer(modifier = Modifier.height(30.dp))

            Text(
                text = "Enter Factory CVC",
                style = MaterialTheme.typography.headlineLarge,
                fontWeight = FontWeight.Bold,
            )

            Text(
                text =
                    "The factory CVC is printed on the back of your TAPSIGNER. " +
                        "Enter it as hexadecimal bytes. " +
                        "For example, printed ASCII 123456 becomes 313233343536.",
                style = MaterialTheme.typography.bodyMedium,
                textAlign = TextAlign.Center,
            )

            TapSignerCvcInput(
                value = factoryCvc,
                onValueChange = { factoryCvc = it },
                label = "Factory CVC (hex)",
                testTag = "tapSignerStarting.factoryCvc",
            )

            Button(
                onClick = {
                    manager.navigate(
                        TapSignerRoute.NewPin(
                            TapSignerNewPinArgs(
                                tapSigner = tapSigner,
                                startingPin = factoryCvc,
                                chainCode = chainCode,
                                action = TapSignerPinAction.SETUP,
                            ),
                        ),
                    )
                },
                enabled = validCvc,
                modifier = Modifier.fillMaxWidth().testTag("tapSignerStarting.continue"),
            ) {
                Text("Continue")
            }

            Spacer(modifier = Modifier.height(30.dp))
        }
    }
}
