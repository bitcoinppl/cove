package org.bitcoinppl.cove.flows.NewWalletFlow.hot_wallet

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.material3.BottomSheetDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.RectangleShape
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.QrCodeScanView
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove_core.AppAlertState
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
        containerColor = Color.Transparent,
        shape = RectangleShape,
        dragHandle = null,
        contentWindowInsets = { WindowInsets(0) },
    ) {
        Box {
            QrCodeScanView(
                showTopBar = false,
                onScanned = { multiFormat ->
                    when (multiFormat) {
                        is MultiFormat.Mnemonic -> {
                            multiFormat.v1.use { mnemonic ->
                                runCatching {
                                    val mnemonicString = mnemonic.words().joinToString(" ")
                                    groupedPlainWordsOf(mnemonic = mnemonicString, groups = GROUPS_OF.toUByte())
                                }.onSuccess { words ->
                                    onWordsScanned(words)
                                }.onFailure {
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
            BottomSheetDefaults.DragHandle(
                modifier = Modifier.align(Alignment.TopCenter).statusBarsPadding(),
                color = Color.White.copy(alpha = 0.5f),
            )
        }
    }
}
