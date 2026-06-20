package org.bitcoinppl.cove.flows.NewWalletFlow

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.R

/**
 * NFC Help content showing instructions for ColdCard Q1 and MK4 hardware wallets.
 * Matches iOS NfcHelpView.swift
 */
@Composable
fun NfcHelpSheet(modifier: Modifier = Modifier) {
    Column(
        modifier =
            modifier
                .fillMaxWidth()
                .padding(22.dp)
                .verticalScroll(rememberScrollState()),
    ) {
        Text(
            text = stringResource(R.string.new_wallet_flow_nfc_help_title),
            fontSize = 28.sp,
            fontWeight = FontWeight.Bold,
            modifier = Modifier.padding(bottom = 18.dp),
        )

        // ColdCard Q1 instructions
        Column(modifier = Modifier.padding(bottom = 32.dp)) {
            Text(
                text = stringResource(R.string.new_wallet_flow_coldcard_q1),
                fontSize = 22.sp,
                fontWeight = FontWeight.Bold,
                modifier = Modifier.padding(bottom = 12.dp),
            )

            InstructionStep(stringResource(R.string.new_wallet_flow_nfc_help_enable_nfc))
            InstructionStep(stringResource(R.string.new_wallet_flow_nfc_help_advanced_tools))
            InstructionStep(stringResource(R.string.new_wallet_flow_nfc_help_export_descriptor))
            InstructionStep(stringResource(R.string.new_wallet_flow_nfc_help_press_enter_nfc))
            InstructionStep(stringResource(R.string.new_wallet_flow_nfc_help_bring_phone_top))
        }

        HorizontalDivider(
            modifier = Modifier.padding(vertical = 16.dp),
            color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.12f),
        )

        // ColdCard MK4 instructions
        Column {
            Text(
                text = stringResource(R.string.new_wallet_flow_coldcard_mk4),
                fontSize = 22.sp,
                fontWeight = FontWeight.Bold,
                modifier = Modifier.padding(bottom = 12.dp),
            )

            InstructionStep(stringResource(R.string.new_wallet_flow_nfc_help_enable_nfc))
            InstructionStep(stringResource(R.string.new_wallet_flow_nfc_help_advanced_tools))
            InstructionStep(stringResource(R.string.new_wallet_flow_nfc_help_export_descriptor))
            InstructionStep(stringResource(R.string.new_wallet_flow_nfc_help_press_enter_nfc))
            InstructionStep(stringResource(R.string.new_wallet_flow_nfc_help_bring_phone_8_button))
        }

        Spacer(modifier = Modifier.height(32.dp))
    }
}

@Composable
private fun InstructionStep(text: String) {
    Text(
        text = text,
        fontSize = 17.sp,
        lineHeight = 24.sp,
        modifier = Modifier.padding(bottom = 8.dp),
    )
}

@Preview(showBackground = true)
@Composable
private fun NfcHelpSheetPreview() {
    NfcHelpSheet()
}
