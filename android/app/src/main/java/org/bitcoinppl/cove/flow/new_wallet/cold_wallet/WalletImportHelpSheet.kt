package org.bitcoinppl.cove.flow.new_wallet.cold_wallet

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
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp

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
                text = "How do get my wallet export QR code?",
                style = MaterialTheme.typography.titleLarge,
                fontWeight = FontWeight.Bold,
            )

            Column(verticalArrangement = Arrangement.spacedBy(32.dp)) {
                // ColdCard Q1
                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text(
                        text = "ColdCard Q1",
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.Bold,
                    )
                    Text("1. Go to 'Advanced / Tools'")
                    Text("2. Export Wallet > Generic JSON")
                    Text("3. Press the 'Enter' button, then the 'QR' button")
                    Text("4. Scan the Generated QR code")
                }

                HorizontalDivider()

                // ColdCard MK3/MK4
                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text(
                        text = "ColdCard MK3/MK4",
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.Bold,
                    )
                    Text("1. Go to 'Advanced / Tools'")
                    Text("2. Export Wallet > Descriptor")
                    Text("3. Press the Enter (✓) and select your wallet type")
                    Text("4. Scan the Generated QR code")
                }

                HorizontalDivider()

                // Sparrow Desktop
                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text(
                        text = "Sparrow Desktop",
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.Bold,
                    )
                    Text("1. Click on Settings, in the left side bar")
                    Text("2. Click on 'Export...' button at the bottom")
                    Text("3. Under 'Output Descriptor' click the 'Show...' button")
                    Text("4. Make sure 'Show BBQr' is selected")
                }

                HorizontalDivider()

                // Other Hardware Wallets
                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text(
                        text = "Other Hardware Wallets",
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.Bold,
                    )
                    Text("1. In your hardware wallet, go to settings")
                    Text("2. Look for 'Export'")
                    Text("3. Select 'Generic JSON', 'Sparrow', 'Electrum', and many other formats should also work")
                    Text("4. Generate QR code")
                    Text("5. Scan the Generated QR code")
                }
            }

            Spacer(modifier = Modifier.height(24.dp))
        }
    }
}
