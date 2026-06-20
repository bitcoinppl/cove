package org.bitcoinppl.cove

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.material3.BottomSheetDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.RectangleShape
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.flows.TapSignerFlow.TapSignerContainer
import org.bitcoinppl.cove.nfc.NfcScanSheet

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun SheetContent(
    state: TaggedItem<AppSheetState>,
    app: AppManager,
    onDismiss: () -> Unit,
) {
    val context = LocalContext.current

    when (state.item) {
        is AppSheetState.Qr -> {
            ModalBottomSheet(
                onDismissRequest = onDismiss,
                sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
                shape = RectangleShape,
                dragHandle = null,
                containerColor = Color.Transparent,
                contentWindowInsets = { WindowInsets(0.dp) },
            ) {
                Box {
                    QrCodeScanView(
                        onScanned = { multiFormat ->
                            app.sheetState = null
                            Scanner.handleMultiFormat(context, multiFormat)
                        },
                        onDismiss = onDismiss,
                        app = app,
                        showTopBar = false,
                    )
                    BottomSheetDefaults.DragHandle(
                        modifier = Modifier.align(Alignment.TopCenter).statusBarsPadding(),
                        color = Color.White.copy(alpha = 0.5f),
                    )
                }
            }
        }
        is AppSheetState.Nfc -> {
            NfcScanSheet(
                app = app,
                onDismiss = onDismiss,
            )
        }
        is AppSheetState.TapSigner -> {
            ModalBottomSheet(
                onDismissRequest = onDismiss,
                sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
            ) {
                TapSignerContainer(
                    route = state.item.route,
                )
            }
        }
    }
}
