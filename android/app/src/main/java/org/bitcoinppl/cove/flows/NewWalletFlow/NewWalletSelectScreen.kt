package org.bitcoinppl.cove.flows.NewWalletFlow

import android.content.ClipboardManager
import android.content.Context
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Image
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.InsertDriveFile
import androidx.compose.material.icons.filled.ContentPaste
import androidx.compose.material.icons.filled.Menu
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.App
import org.bitcoinppl.cove.AppAlertState
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.ForceLightStatusBarIcons
import org.bitcoinppl.cove.ui.theme.title3
import org.bitcoinppl.cove.views.DotMenuView
import org.bitcoinppl.cove.views.ImageButton
import org.bitcoinppl.cove_core.Wallet
import org.bitcoinppl.cove_core.WalletException

@Preview(showBackground = true, backgroundColor = 0xFF0D1B2A)
@Composable
private fun NewWalletSelectScreenPreview() {
    val snack = remember { SnackbarHostState() }
    NewWalletSelectScreen(
        app = App,
        onBack = {},
        canGoBack = false,
        onOpenNewHotWallet = {},
        onOpenQrScan = {},
        onOpenNfcScan = {},
        snackbarHostState = snack,
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun NewWalletSelectScreen(
    app: AppManager,
    onBack: () -> Unit,
    canGoBack: Boolean,
    onOpenNewHotWallet: () -> Unit,
    onOpenQrScan: () -> Unit,
    onOpenNfcScan: () -> Unit,
    snackbarHostState: SnackbarHostState = remember { SnackbarHostState() },
) {
    var showHardwareWalletSheet by remember { mutableStateOf(false) }
    var showNfcHelpSheet by remember { mutableStateOf(false) }
    var nfcCalled by remember { mutableStateOf(false) }
    val context = LocalContext.current
    val scope = rememberCoroutineScope()

    // function to trigger NFC scan and show help after delay (matching iOS behavior)
    fun triggerNfcScan() {
        onOpenNfcScan()
        scope.launch {
            delay(800)
            nfcCalled = true
        }
    }

    fun importWallet(content: String) {
        try {
            val wallet = Wallet.newFromXpub(xpub = content.trim())
            val id = wallet.id()
            android.util.Log.d("NewWalletSelectScreen", "Imported Wallet: $id")

            app.popRoute()
            app.rust.selectWallet(id = id)
            app.alertState = TaggedItem(AppAlertState.ImportedSuccessfully)
        } catch (e: WalletException.MultiFormat) {
            app.popRoute()
            app.alertState =
                TaggedItem(
                    AppAlertState.ErrorImportingHardwareWallet(
                        message = e.v1.toString(),
                    ),
                )
        } catch (e: WalletException.WalletAlreadyExists) {
            app.popRoute()
            app.alertState = TaggedItem(AppAlertState.DuplicateWallet(e.v1))
        } catch (e: Exception) {
            android.util.Log.w("NewWalletSelectScreen", "Error importing hardware wallet: $e")
            app.popRoute()
            app.alertState =
                TaggedItem(
                    AppAlertState.ErrorImportingHardwareWallet(
                        message = e.message ?: "Unknown error",
                    ),
                )
        }
    }

    val filePickerLauncher =
        rememberLauncherForActivityResult(
            contract = ActivityResultContracts.GetContent(),
        ) { uri ->
            uri?.let {
                // read file content and import wallet
                context.contentResolver.openInputStream(it)?.use { stream ->
                    val content = stream.bufferedReader().readText()
                    importWallet(content)
                }
            }
        }

    fun pasteFromClipboard(): String? {
        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        return clipboard.primaryClip
            ?.getItemAt(0)
            ?.text
            ?.toString()
    }

    // force white status bar icons for midnight blue background
    ForceLightStatusBarIcons()

    Scaffold(containerColor = CoveColor.midnightBlue, topBar = {
        CenterAlignedTopAppBar(
            colors =
                TopAppBarDefaults.centerAlignedTopAppBarColors(
                    containerColor = Color.Transparent,
                    titleContentColor = Color.White,
                    actionIconContentColor = Color.White,
                    navigationIconContentColor = Color.White,
                ),
            title = {
                Text(
                    stringResource(R.string.title_wallet_add),
                    style = MaterialTheme.typography.titleMedium,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            },
            navigationIcon = {
                if (canGoBack) {
                    IconButton(onClick = onBack) {
                        Icon(
                            imageVector = Icons.AutoMirrored.Default.ArrowBack,
                            contentDescription = "Back",
                        )
                    }
                } else {
                    IconButton(onClick = onBack) {
                        Icon(
                            imageVector = Icons.Filled.Menu,
                            contentDescription = "Menu",
                        )
                    }
                }
            },
            actions = {
                IconButton(onClick = onOpenQrScan) {
                    Icon(
                        painter = painterResource(id = R.drawable.icon_qr_code),
                        contentDescription = "Scan QR",
                    )
                }
                IconButton(onClick = { triggerNfcScan() }) {
                    Icon(
                        painter = painterResource(id = R.drawable.icon_contactless),
                        contentDescription = "NFC",
                    )
                }
            },
        )
    }, snackbarHost = { SnackbarHost(snackbarHostState) }) { padding ->

        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(padding),
        ) {
            Image(
                painter = painterResource(id = R.drawable.image_chain_code_pattern_horizontal),
                contentDescription = null,
                contentScale = ContentScale.Crop,
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .padding(top = 36.dp)
                        .heightIn(min = 0.dp, max = (0.75f * 720).dp)
                        .align(Alignment.TopCenter),
            )

            Row(
                modifier =
                    Modifier
                        .align(Alignment.BottomCenter)
                        .padding(horizontal = 20.dp, vertical = 16.dp)
                        .fillMaxWidth(),
            ) {
                Column(
                    modifier = Modifier.fillMaxWidth(),
                    verticalArrangement = Arrangement.spacedBy(28.dp),
                ) {
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        DotMenuView(
                            count = 4,
                            currentIndex = 0,
                        )
                        Spacer(Modifier.weight(1f))
                    }

                    Text(
                        text = stringResource(R.string.label_wallet_add_new_wallet),
                        color = Color.White,
                        fontSize = 34.sp,
                        fontWeight = FontWeight.SemiBold,
                        lineHeight = 38.sp,
                    )

                    HorizontalDivider(
                        color = Color.White.copy(alpha = 0.35f),
                        thickness = 1.dp,
                    )

                    Row(
                        horizontalArrangement = Arrangement.spacedBy(16.dp),
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        ImageButton(
                            text = stringResource(R.string.btn_hardware_wallet),
                            leadingIcon = painterResource(R.drawable.icon_currency_bitcoin),
                            onClick = {
                                showHardwareWalletSheet = true
                            },
                            colors =
                                ButtonDefaults.buttonColors(
                                    containerColor = CoveColor.btnPrimary,
                                    contentColor = CoveColor.midnightBlue,
                                ),
                            modifier = Modifier.weight(1f),
                        )

                        ImageButton(
                            text = stringResource(R.string.btn_on_this_device),
                            leadingIcon = painterResource(R.drawable.icon_phone_device),
                            onClick = onOpenNewHotWallet,
                            colors =
                                ButtonDefaults.buttonColors(
                                    containerColor = CoveColor.btnPrimary,
                                    contentColor = CoveColor.midnightBlue,
                                ),
                            modifier = Modifier.weight(1f),
                        )
                    }

                    // NFC Help button - appears after NFC is called (matching iOS behavior)
                    if (nfcCalled) {
                        Text(
                            text = "NFC Help",
                            color = Color.White,
                            modifier =
                                Modifier
                                    .clickable { showNfcHelpSheet = true }
                                    .padding(vertical = 8.dp)
                                    .fillMaxWidth(),
                            textAlign = TextAlign.Center,
                        )
                    }
                }
            }
        }
    }

    // NFC Help sheet
    if (showNfcHelpSheet) {
        ModalBottomSheet(
            onDismissRequest = { showNfcHelpSheet = false },
        ) {
            NfcHelpSheet()
        }
    }

    if (showHardwareWalletSheet) {
        ModalBottomSheet(
            onDismissRequest = { showHardwareWalletSheet = false },
        ) {
            Column(
                modifier =
                    Modifier
                        .padding(horizontal = 16.dp)
                        .padding(bottom = 32.dp),
            ) {
                Text(
                    text = "Import Hardware Wallet",
                    style = MaterialTheme.typography.title3,
                    modifier = Modifier.padding(bottom = 16.dp),
                )

                // QR Code option
                ListItem(
                    headlineContent = { Text("QR Code") },
                    supportingContent = { Text("Scan descriptor QR code") },
                    leadingContent = {
                        Icon(
                            painter = painterResource(R.drawable.icon_qr_code),
                            contentDescription = null,
                        )
                    },
                    modifier =
                        Modifier.clickable {
                            showHardwareWalletSheet = false
                            onOpenQrScan()
                        },
                )

                // File option
                ListItem(
                    headlineContent = { Text("File") },
                    supportingContent = { Text("Import from file") },
                    leadingContent = {
                        Icon(
                            imageVector = Icons.AutoMirrored.Filled.InsertDriveFile,
                            contentDescription = null,
                        )
                    },
                    modifier =
                        Modifier.clickable {
                            showHardwareWalletSheet = false
                            filePickerLauncher.launch("*/*")
                        },
                )

                // NFC option
                ListItem(
                    headlineContent = { Text("NFC") },
                    supportingContent = { Text("Tap hardware wallet") },
                    leadingContent = {
                        Icon(
                            painter = painterResource(R.drawable.icon_contactless),
                            contentDescription = null,
                        )
                    },
                    modifier =
                        Modifier.clickable {
                            showHardwareWalletSheet = false
                            triggerNfcScan()
                        },
                )

                // Paste option
                ListItem(
                    headlineContent = { Text("Paste") },
                    supportingContent = { Text("Paste from clipboard") },
                    leadingContent = {
                        Icon(
                            imageVector = Icons.Default.ContentPaste,
                            contentDescription = null,
                        )
                    },
                    modifier =
                        Modifier.clickable {
                            showHardwareWalletSheet = false
                            pasteFromClipboard()?.let { content ->
                                importWallet(content)
                            }
                        },
                )
            }
        }
    }
}
