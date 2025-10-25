package org.bitcoinppl.cove.tapsigner

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
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
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove_core.TapSignerNewPinArgs
import org.bitcoinppl.cove_core.TapSignerPinAction
import org.bitcoinppl.cove_core.TapSignerRoute

/**
 * starting (factory) PIN entry screen
 * first step in TapSigner setup flow
 */
@Composable
fun TapSignerStartingPinView(
    app: AppManager,
    manager: TapSignerManager,
    tapSigner: org.bitcoinppl.cove_core.tapcard.TapSigner,
    chainCode: String?,
    modifier: Modifier = Modifier,
) {
    var startingPin by remember { mutableStateOf("") }

    // reset PIN when screen appears
    LaunchedEffect(Unit) {
        startingPin = ""
    }

    // navigate to new PIN screen when 6 digits entered
    LaunchedEffect(startingPin) {
        if (startingPin.length == 6) {
            delay(200)
            manager.navigate(
                TapSignerRoute.NewPin(
                    TapSignerNewPinArgs(
                        tapSigner = tapSigner,
                        startingPin = startingPin,
                        chainCode = chainCode,
                        action = TapSignerPinAction.SETUP,
                    ),
                ),
            )
        }
    }

    Column(
        modifier =
            modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState()),
    ) {
        // header with card image
        Box(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .background(Color(0xFF3A4254)),
        ) {
            Column {
                // back button
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

                // TapSigner card image
                // TODO: replace with actual TapSigner card image
                // Image(
                //     painter = painterResource(R.drawable.tapsigner_card),
                //     contentDescription = "TapSigner Card",
                //     modifier = Modifier.offset(y = 10.dp)
                // )
                Spacer(modifier = Modifier.height(100.dp))
            }
        }

        // content
        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(20.dp),
        ) {
            Spacer(modifier = Modifier.height(30.dp))

            // title
            Text(
                text = "Enter Starting PIN",
                style = MaterialTheme.typography.headlineLarge,
                fontWeight = FontWeight.Bold,
            )

            // description
            Text(
                text =
                    "The starting PIN is the 6 digit numeric PIN found of the back of your TAPSIGNER",
                style = MaterialTheme.typography.bodyMedium,
                textAlign = TextAlign.Center,
            )

            Spacer(modifier = Modifier.height(10.dp))

            // PIN circles
            Box(
                modifier = Modifier.fillMaxWidth(),
                contentAlignment = Alignment.Center,
            ) {
                PinCirclesView(pinLength = startingPin.length)
            }

            // hidden text field
            HiddenPinTextField(
                value = startingPin,
                onValueChange = { startingPin = it },
            )
        }
    }
}
