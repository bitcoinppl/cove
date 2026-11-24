// file level so the sheet's signature keeps matching its detekt baseline entries
@file:OptIn(ExperimentalComposeUiApi::class)

package org.bitcoinppl.cove.sheets

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.flows.SendFlow.SendFlowManager
import org.bitcoinppl.cove.flows.SendFlow.SendFlowPresenter
import org.bitcoinppl.cove.ui.theme.coveColors
import org.bitcoinppl.cove.ui.theme.title3
import org.bitcoinppl.cove.views.feerate.FeeRateOptionRow
import org.bitcoinppl.cove.views.feerate.approximateFiat
import org.bitcoinppl.cove.views.lockaware.ModalBottomSheet
import org.bitcoinppl.cove_core.types.FeeRateOptionWithTotalFee
import org.bitcoinppl.cove_core.types.FeeRateOptionsWithTotalFee

/** fee rate selector sheet - displays fast/medium/slow fee options */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun FeeRateSelectorSheet(
    app: AppManager,
    walletManager: WalletManager,
    sendFlowManager: SendFlowManager,
    presenter: SendFlowPresenter,
    feeOptions: FeeRateOptionsWithTotalFee,
    selectedOption: FeeRateOptionWithTotalFee,
    onSelectFee: (FeeRateOptionWithTotalFee) -> Unit,
    onUpdateFeeOptions: (FeeRateOptionsWithTotalFee) -> Unit,
    onDismiss: () -> Unit,
) {
    var showCustomFeeSheet by remember { mutableStateOf(false) }
    var currentFeeOptions by remember(feeOptions) { mutableStateOf(feeOptions) }

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        containerColor = MaterialTheme.colorScheme.surface,
    ) {
        // a sheet is its own window, so its tags need exposing to UI tests here too
        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .semantics { testTagsAsResourceId = true }
                    .padding(horizontal = 16.dp)
                    .padding(bottom = 32.dp),
        ) {
            // title
            Text(
                text = "Network Fee",
                style = MaterialTheme.typography.title3,
                fontWeight = FontWeight.Bold,
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .padding(vertical = 8.dp),
                textAlign = androidx.compose.ui.text.style.TextAlign.Center,
            )

            Spacer(modifier = Modifier.height(20.dp))

            // fast option
            SendFlowFeeOptionRow(
                app = app,
                manager = walletManager,
                feeOption = currentFeeOptions.fast(),
                isSelected = selectedOption.feeSpeed() == currentFeeOptions.fast().feeSpeed(),
                onSelect = {
                    onSelectFee(currentFeeOptions.fast())
                    onDismiss()
                },
            )

            Spacer(modifier = Modifier.height(12.dp))

            // medium option
            SendFlowFeeOptionRow(
                app = app,
                manager = walletManager,
                feeOption = currentFeeOptions.medium(),
                isSelected = selectedOption.feeSpeed() == currentFeeOptions.medium().feeSpeed(),
                onSelect = {
                    onSelectFee(currentFeeOptions.medium())
                    onDismiss()
                },
            )

            Spacer(modifier = Modifier.height(12.dp))

            // slow option
            SendFlowFeeOptionRow(
                app = app,
                manager = walletManager,
                feeOption = currentFeeOptions.slow(),
                isSelected = selectedOption.feeSpeed() == currentFeeOptions.slow().feeSpeed(),
                onSelect = {
                    onSelectFee(currentFeeOptions.slow())
                    onDismiss()
                },
            )

            // custom option if exists
            currentFeeOptions.custom()?.let { customOption ->
                Spacer(modifier = Modifier.height(12.dp))
                SendFlowFeeOptionRow(
                    app = app,
                    manager = walletManager,
                    feeOption = customOption,
                    isSelected = selectedOption.feeSpeed() == customOption.feeSpeed(),
                    onSelect = {
                        onSelectFee(customOption)
                        onDismiss()
                    },
                )
            }

            Spacer(modifier = Modifier.height(20.dp))

            // customize fee button
            Button(
                onClick = { showCustomFeeSheet = true },
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 24.dp),
                colors =
                    ButtonDefaults.buttonColors(
                        containerColor = MaterialTheme.coveColors.midnightBtn,
                        contentColor = Color.White,
                    ),
                shape = RoundedCornerShape(10.dp),
            ) {
                Text(
                    text = "Customize Fee",
                    style = MaterialTheme.typography.bodyMedium,
                    fontWeight = FontWeight.SemiBold,
                    modifier = Modifier.padding(vertical = 4.dp),
                )
            }
        }
    }

    // custom fee sheet
    if (showCustomFeeSheet) {
        CustomFeeRateSheet(
            app = app,
            walletManager = walletManager,
            sendFlowManager = sendFlowManager,
            presenter = presenter,
            feeOptions = currentFeeOptions,
            selectedOption = selectedOption,
            onUpdateFeeOptions = { newOptions, newSelected ->
                currentFeeOptions = newOptions
                onUpdateFeeOptions(newOptions)
                onSelectFee(newSelected)
                showCustomFeeSheet = false
            },
            onDismiss = {
                showCustomFeeSheet = false
            },
        )
    }
}

/** A fee row of the send flow, with its total in fiat once prices load */
@Composable
private fun SendFlowFeeOptionRow(
    app: AppManager,
    manager: WalletManager,
    feeOption: FeeRateOptionWithTotalFee,
    isSelected: Boolean,
    onSelect: () -> Unit,
) {
    val fiatAmount =
        remember(feeOption, app.prices) {
            feeOption.totalFee()?.let { approximateFiat(app.prices, manager, it) }
        }

    FeeRateOptionRow(
        feeOption = feeOption,
        isSelected = isSelected,
        fiatAmount = fiatAmount,
        testTagPrefix = "sendFlow.feeOption",
        onSelect = onSelect,
    )
}
