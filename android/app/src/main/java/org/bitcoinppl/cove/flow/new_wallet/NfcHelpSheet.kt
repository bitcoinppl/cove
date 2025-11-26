package org.bitcoinppl.cove.flow.new_wallet

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
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

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
            text = "How do I import using NFC?",
            fontSize = 24.sp,
            fontWeight = FontWeight.Bold,
            modifier = Modifier.padding(bottom = 18.dp),
        )

        // ColdCard Q1 instructions
        Column(modifier = Modifier.padding(bottom = 32.dp)) {
            Text(
                text = "ColdCard Q1",
                fontSize = 20.sp,
                fontWeight = FontWeight.Bold,
                modifier = Modifier.padding(bottom = 12.dp),
            )

            InstructionStep("1. Enable NFC by going to 'Settings' > 'Hardware On/Off' > 'NFC Sharing' ")
            InstructionStep("2. Go to 'Advanced / Tools'")
            InstructionStep("3. Export Wallet > 'Descriptor' > 'Segwit P2WPKH'")
            InstructionStep("4. Press the 'Enter' button, then the 'NFC' button")
            InstructionStep("5. Bring the phone to the top of the screen")
        }

        HorizontalDivider(
            modifier = Modifier.padding(vertical = 16.dp),
            color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.12f),
        )

        // ColdCard MK4 instructions
        Column {
            Text(
                text = "ColdCard MK4",
                fontSize = 20.sp,
                fontWeight = FontWeight.Bold,
                modifier = Modifier.padding(bottom = 12.dp),
            )

            InstructionStep("1. Enable NFC by going to 'Settings' > 'Hardware On/Off' > 'NFC Sharing' ")
            InstructionStep("2. Go to 'Advanced / Tools'")
            InstructionStep("3. Export Wallet > 'Descriptor' > 'Segwit P2WPKH'")
            InstructionStep("4. Press the 'Enter' button, then the 'NFC' button")
            InstructionStep("5. Bring the phone to the to the coldcard near the 8 button")
        }

        Spacer(modifier = Modifier.height(32.dp))
    }
}

@Composable
private fun InstructionStep(text: String) {
    Text(
        text = text,
        fontSize = 16.sp,
        lineHeight = 24.sp,
        modifier = Modifier.padding(bottom = 8.dp),
    )
}

@Preview(showBackground = true)
@Composable
private fun NfcHelpSheetPreview() {
    NfcHelpSheet()
}
