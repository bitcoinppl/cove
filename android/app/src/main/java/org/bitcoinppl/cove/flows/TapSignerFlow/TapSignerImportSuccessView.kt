package org.bitcoinppl.cove.flows.TapSignerFlow

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Error
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove_core.types.WalletId
import org.bitcoinppl.cove.Log

private suspend fun persistWallet(
    tapSigner: org.bitcoinppl.cove_core.tapcard.TapSigner,
    deriveInfo: org.bitcoinppl.cove_core.DeriveInfo,
): WalletId =
    withContext(Dispatchers.IO) {
        // the manager only persists the wallet; until it closes, it holds the wallet's lifecycle
        // and the app can't load the wallet when Continue selects it
        WalletManager.fromTapSigner(tapSigner, deriveInfo).use { it.id }
    }

/** Where saving the imported wallet stands */
internal sealed interface TapSignerImportSaveState {
    data object Saving : TapSignerImportSaveState

    data class Failed(
        val message: String,
    ) : TapSignerImportSaveState

    data class Saved(
        val walletId: WalletId,
    ) : TapSignerImportSaveState
}

/**
 * import success screen
 * displays after successful TapSigner wallet import
 */
@Composable
fun TapSignerImportSuccessView(
    app: AppManager,
    manager: TapSignerManager,
    tapSigner: org.bitcoinppl.cove_core.tapcard.TapSigner,
    deriveInfo: org.bitcoinppl.cove_core.DeriveInfo,
    modifier: Modifier = Modifier,
) {
    var saveState: TapSignerImportSaveState by remember { mutableStateOf(TapSignerImportSaveState.Saving) }
    val scope = rememberCoroutineScope()

    suspend fun save() {
        saveState = TapSignerImportSaveState.Saving
        saveState =
            try {
                TapSignerImportSaveState.Saved(persistWallet(tapSigner, deriveInfo))
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                Log.e("TapSignerImportSuccess", "Failed to save TapSigner wallet")
                TapSignerImportSaveState.Failed("Failed to save wallet")
            }
    }

    // save wallet on appear
    LaunchedEffect(tapSigner, deriveInfo) { save() }

    TapSignerImportSuccessContent(
        saveState = saveState,
        onCancel = { app.sheetState = null },
        onRetry = { scope.launch { save() } },
        onContinue = { walletId ->
            app.selectWallet(walletId)
            app.sheetState = null
        },
        modifier = modifier,
    )
}

/** What the import success sheet shows for [saveState] */
@Composable
internal fun TapSignerImportSuccessContent(
    saveState: TapSignerImportSaveState,
    onCancel: () -> Unit,
    onRetry: () -> Unit,
    onContinue: (WalletId) -> Unit,
    modifier: Modifier = Modifier,
) {
    TapSignerSheetLayout(
        testTag = "tapSignerImportSuccess",
        modifier = modifier.padding(horizontal = 16.dp),
        header = {
            Row(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .padding(top = 20.dp),
                horizontalArrangement = Arrangement.Start,
            ) {
                TextButton(onClick = onCancel) {
                    Text("Cancel", fontWeight = FontWeight.SemiBold)
                }
            }
        },
        content = {
            when (saveState) {
                TapSignerImportSaveState.Saving -> ImportSavingMessage()
                is TapSignerImportSaveState.Failed -> ImportFailedMessage(saveState.message, onRetry)
                is TapSignerImportSaveState.Saved -> ImportCompleteMessage()
            }
        },
        action = {
            val savedWalletId = (saveState as? TapSignerImportSaveState.Saved)?.walletId
            Button(
                onClick = { savedWalletId?.let(onContinue) },
                enabled = savedWalletId != null,
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .padding(bottom = 30.dp)
                        .testTag("tapSignerImportSuccess.continue"),
            ) {
                Text("Continue")
            }
        },
    )
}

@Composable
private fun ImportSavingMessage() {
    CircularProgressIndicator(modifier = Modifier.size(60.dp))
    Spacer(modifier = Modifier.height(20.dp))
    Text(
        text = "Saving wallet...",
        style = MaterialTheme.typography.bodyLarge,
        color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.8f),
    )
}

@Composable
private fun ImportFailedMessage(
    message: String,
    onRetry: () -> Unit,
) {
    Icon(
        imageVector = Icons.Default.Error,
        contentDescription = "Error",
        modifier = Modifier.size(100.dp),
        tint = MaterialTheme.colorScheme.error,
    )

    Spacer(modifier = Modifier.height(20.dp))

    Column(
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text(
            text = "Import Failed",
            style = MaterialTheme.typography.headlineLarge,
            fontWeight = FontWeight.Bold,
        )

        Text(
            text = message,
            style = MaterialTheme.typography.bodyMedium,
            textAlign = TextAlign.Center,
            color = MaterialTheme.colorScheme.error,
        )
    }

    Spacer(modifier = Modifier.height(20.dp))

    Button(onClick = onRetry) {
        Text("Retry")
    }
}

@Composable
private fun ImportCompleteMessage() {
    Icon(
        imageVector = Icons.Default.CheckCircle,
        contentDescription = "Success",
        modifier = Modifier.size(100.dp),
        tint = Color.Green,
    )

    Spacer(modifier = Modifier.height(20.dp))

    Column(
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text(
            text = "Import Complete",
            style = MaterialTheme.typography.headlineLarge,
            fontWeight = FontWeight.Bold,
        )

        Text(
            text = "Your TAPSIGNER wallet has been imported successfully.",
            style = MaterialTheme.typography.bodyMedium,
            textAlign = TextAlign.Center,
            color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.8f),
        )
    }
}
