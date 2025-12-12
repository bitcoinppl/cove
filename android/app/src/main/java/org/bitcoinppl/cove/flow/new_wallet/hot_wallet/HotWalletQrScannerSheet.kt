package org.bitcoinppl.cove.flow.new_wallet.hot_wallet

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import org.bitcoinppl.cove.AppAlertState
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.QrCodeScanView
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove_core.MultiFormat
import org.bitcoinppl.cove_core.groupedPlainWordsOf

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun QrScannerSheet(
    app: AppManager,
    onDismiss: () -> Unit,
    onWordsScanned: (List<List<String>>) -> Unit,
) {
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        containerColor = Color.Black,
    ) {
        QrCodeScanView(
            showTopBar = false,
            onScanned = { multiFormat ->
                when (multiFormat) {
                    is MultiFormat.Mnemonic -> {
                        val mnemonicString = multiFormat.v1.words().joinToString(" ")
                        val words = groupedPlainWordsOf(mnemonic = mnemonicString, groups = GROUPS_OF.toUByte())
                        onWordsScanned(words)
                    }
                    else -> {
                        onDismiss()
                        app.alertState =
                            TaggedItem(
                                AppAlertState.General(
                                    title = "Invalid QR Code",
                                    message = "Please scan a valid seed phrase QR code",
                                ),
                            )
                    }
                }
            },
            onDismiss = onDismiss,
            app = app,
            modifier = Modifier.fillMaxSize(),
        )
    }
}
