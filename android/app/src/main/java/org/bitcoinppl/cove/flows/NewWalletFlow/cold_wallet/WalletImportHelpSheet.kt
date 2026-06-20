package org.bitcoinppl.cove.flows.NewWalletFlow.cold_wallet

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.ui.theme.title3

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun WalletImportHelpSheet(onDismiss: () -> Unit) {
    ModalBottomSheet(onDismissRequest = onDismiss) {
        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(24.dp)
                    .verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(24.dp),
        ) {
            Text(
                text = stringResource(R.string.new_wallet_flow_wallet_export_qr_help_title),
                style = MaterialTheme.typography.title3,
                fontWeight = FontWeight.Bold,
            )

            Column(verticalArrangement = Arrangement.spacedBy(32.dp)) {
                // ColdCard Q1
                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text(
                        text = stringResource(R.string.new_wallet_flow_coldcard_q1),
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.Bold,
                    )
                    Text(stringResource(R.string.new_wallet_flow_qr_help_advanced_tools))
                    Text(stringResource(R.string.new_wallet_flow_qr_help_export_generic_json))
                    Text(stringResource(R.string.new_wallet_flow_qr_help_press_enter_qr))
                    Text(stringResource(R.string.new_wallet_flow_qr_help_scan_generated_qr))
                }

                HorizontalDivider()

                // ColdCard MK3/MK4
                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text(
                        text = stringResource(R.string.new_wallet_flow_coldcard_mk3_mk4),
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.Bold,
                    )
                    Text(stringResource(R.string.new_wallet_flow_qr_help_advanced_tools))
                    Text(stringResource(R.string.new_wallet_flow_qr_help_export_descriptor))
                    Text(stringResource(R.string.new_wallet_flow_qr_help_press_enter_select_wallet_type))
                    Text(stringResource(R.string.new_wallet_flow_qr_help_scan_generated_qr))
                }

                HorizontalDivider()

                // Sparrow Desktop
                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text(
                        text = stringResource(R.string.new_wallet_flow_sparrow_desktop),
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.Bold,
                    )
                    Text(stringResource(R.string.new_wallet_flow_qr_help_sparrow_settings))
                    Text(stringResource(R.string.new_wallet_flow_qr_help_sparrow_export))
                    Text(stringResource(R.string.new_wallet_flow_qr_help_sparrow_show_descriptor))
                    Text(stringResource(R.string.new_wallet_flow_qr_help_sparrow_show_bbqr))
                    Text(stringResource(R.string.new_wallet_flow_qr_help_scan_generated_qr_lower))
                }

                HorizontalDivider()

                // Other Hardware Wallets
                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text(
                        text = stringResource(R.string.new_wallet_flow_other_hardware_wallets),
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.Bold,
                    )
                    Text(stringResource(R.string.new_wallet_flow_qr_help_hardware_settings))
                    Text(stringResource(R.string.new_wallet_flow_qr_help_look_for_export))
                    Text(stringResource(R.string.new_wallet_flow_qr_help_select_export_format))
                    Text(stringResource(R.string.new_wallet_flow_qr_help_generate_qr))
                    Text(stringResource(R.string.new_wallet_flow_qr_help_scan_generated_qr_step_5))
                }
            }

            Spacer(modifier = Modifier.height(24.dp))
        }
    }
}
