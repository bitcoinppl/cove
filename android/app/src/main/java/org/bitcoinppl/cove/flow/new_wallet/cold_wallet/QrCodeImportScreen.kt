package org.bitcoinppl.cove.flow.new_wallet.cold_wallet

import android.Manifest
import android.util.Log
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalLifecycleOwner
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import com.google.accompanist.permissions.ExperimentalPermissionsApi
import com.google.accompanist.permissions.PermissionState
import com.google.accompanist.permissions.isGranted
import com.google.accompanist.permissions.rememberPermissionState
import com.google.mlkit.vision.barcode.BarcodeScanning
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.common.InputImage
import org.bitcoinppl.cove.AppAlertState
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove_core.MultiQr
import org.bitcoinppl.cove_core.Wallet
import org.bitcoinppl.cove_core.WalletException
import java.util.concurrent.Executors

@OptIn(ExperimentalPermissionsApi::class, ExperimentalMaterial3Api::class)
@Composable
fun QrCodeImportScreen(app: AppManager, modifier: Modifier = Modifier) {
    val cameraPermissionState = rememberPermissionState(Manifest.permission.CAMERA)
    var showHelp by remember { mutableStateOf(false) }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { },
                navigationIcon = {
                    IconButton(onClick = { app.popRoute() }) {
                        Icon(
                            Icons.AutoMirrored.Filled.ArrowBack,
                            contentDescription = "Back",
                            tint = Color.White,
                        )
                    }
                },
                actions = {
                    TextButton(onClick = { showHelp = true }) {
                        Text(
                            text = "?",
                            color = Color.White,
                            style = MaterialTheme.typography.titleLarge,
                            fontWeight = FontWeight.Medium,
                        )
                    }
                },
                colors =
                    TopAppBarDefaults.topAppBarColors(
                        containerColor = Color.Transparent,
                    ),
            )
        },
        containerColor = Color.Black,
        modifier = modifier.fillMaxSize(),
    ) { paddingValues ->
        Box(modifier = Modifier.fillMaxSize().padding(paddingValues)) {
            when {
                cameraPermissionState.status.isGranted -> {
                    QrScannerContent(
                        app = app,
                        modifier = Modifier.fillMaxSize(),
                    )
                }
                else -> {
                    PermissionDeniedContent(
                        app = app,
                        permissionState = cameraPermissionState,
                        modifier = Modifier.fillMaxSize(),
                    )
                }
            }
        }
    }

    if (showHelp) {
        HelpSheet(onDismiss = { showHelp = false })
    }
}

@OptIn(ExperimentalPermissionsApi::class)
@Composable
private fun PermissionDeniedContent(
    app: AppManager,
    permissionState: PermissionState,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier = modifier.padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Text(
            text = "Camera Access Required",
            style = MaterialTheme.typography.titleMedium,
            fontWeight = FontWeight.Bold,
            color = Color.White,
        )

        Spacer(modifier = Modifier.height(16.dp))

        Text(
            text = "Please allow camera access in Settings to use this feature.",
            style = MaterialTheme.typography.bodyMedium,
            color = Color.White.copy(alpha = 0.7f),
        )

        Spacer(modifier = Modifier.height(24.dp))

        Button(
            onClick = {
                permissionState.launchPermissionRequest()
            },
        ) {
            Text("Grant Permission")
        }

        Spacer(modifier = Modifier.height(12.dp))

        TextButton(
            onClick = {
                app.popRoute()
                app.alertState = TaggedItem(AppAlertState.NoCameraPermission)
            },
        ) {
            Text("Cancel", color = Color.White)
        }
    }
}

@Composable
@androidx.camera.core.ExperimentalGetImage
private fun QrScannerContent(
    app: AppManager,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val lifecycleOwner = LocalLifecycleOwner.current

    var multiQr by remember { mutableStateOf<MultiQr?>(null) }
    var scanComplete by remember { mutableStateOf(false) }
    var totalParts by remember { mutableStateOf<UInt?>(null) }
    var partsLeft by remember { mutableStateOf<UInt?>(null) }
    var scannedCode by remember { mutableStateOf<String?>(null) }

    val partsScanned =
        remember(totalParts, partsLeft) {
            totalParts?.let { total ->
                partsLeft?.let { left ->
                    (total - left).toInt()
                }
            }
        }

    // handle wallet import when scannedCode changes
    LaunchedEffect(scannedCode) {
        scannedCode?.let { xpub ->
            try {
                val wallet = Wallet.newFromXpub(xpub = xpub)
                val id = wallet.id()
                Log.d("QrCodeImportScreen", "Imported Wallet: $id")

                app.rust.selectWallet(id = id)
                app.alertState =
                    TaggedItem(
                        AppAlertState.General(
                            title = "Success",
                            message = "Imported Wallet Successfully",
                        ),
                    )
                app.popRoute()
            } catch (e: WalletException.MultiFormat) {
                app.popRoute()
                app.alertState =
                    TaggedItem(
                        AppAlertState.ErrorImportingHardwareWallet(
                            message = e.v1.toString(),
                        ),
                    )
            } catch (e: WalletException.WalletAlreadyExists) {
                try {
                    app.rust.selectWallet(id = e.v1)
                    app.alertState =
                        TaggedItem(
                            AppAlertState.General(
                                title = "Success",
                                message = "Wallet already exists: ${e.v1}",
                            ),
                        )
                    app.popRoute()
                } catch (selectError: Exception) {
                    app.popRoute()
                    app.alertState =
                        TaggedItem(
                            AppAlertState.ErrorImportingHardwareWallet(
                                message = "Unable to select wallet",
                            ),
                        )
                }
            } catch (e: Exception) {
                Log.w("QrCodeImportScreen", "Error importing hardware wallet: $e")
                app.popRoute()
                app.alertState =
                    TaggedItem(
                        AppAlertState.ErrorImportingHardwareWallet(
                            message = e.message ?: "Unknown error",
                        ),
                    )
            }
        }
    }

    val barcodeScanner = remember { BarcodeScanning.getClient() }
    val executor = remember { Executors.newSingleThreadExecutor() }
    val cameraProviderRef = remember { mutableStateOf<ProcessCameraProvider?>(null) }
    val previewRef = remember { mutableStateOf<Preview?>(null) }
    val analysisRef = remember { mutableStateOf<ImageAnalysis?>(null) }

    Box(modifier = modifier) {
        if (!scanComplete) {
            // camera preview
            AndroidView(
                factory = { ctx ->
                    val previewView = PreviewView(ctx)
                    val cameraProviderFuture = ProcessCameraProvider.getInstance(ctx)

                    cameraProviderFuture.addListener({
                        val cameraProvider = cameraProviderFuture.get()
                        cameraProviderRef.value = cameraProvider

                        val preview =
                            Preview.Builder().build().also {
                                it.setSurfaceProvider(previewView.surfaceProvider)
                            }
                        previewRef.value = preview

                        val imageAnalysis =
                            ImageAnalysis.Builder()
                                .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                                .build()
                                .also { analysis ->
                                    analysis.setAnalyzer(executor) { imageProxy ->
                                        val mediaImage = imageProxy.image
                                        if (mediaImage != null) {
                                            val image =
                                                InputImage.fromMediaImage(
                                                    mediaImage,
                                                    imageProxy.imageInfo.rotationDegrees,
                                                )

                                            val mainExecutor = ContextCompat.getMainExecutor(ctx)
                                            barcodeScanner.process(image)
                                                .addOnSuccessListener(mainExecutor) { barcodes ->
                                                    for (barcode in barcodes) {
                                                        if (barcode.format == Barcode.FORMAT_QR_CODE) {
                                                            handleQrCode(
                                                                barcode = barcode,
                                                                multiQr = multiQr,
                                                                onMultiQrUpdate = { multiQr = it },
                                                                onTotalPartsUpdate = { totalParts = it },
                                                                onPartsLeftUpdate = { partsLeft = it },
                                                                onScanComplete = {
                                                                    scanComplete = true
                                                                    scannedCode = it
                                                                },
                                                                onError = { error ->
                                                                    Log.e("QrCodeImportScreen", "Error: $error")
                                                                    app.alertState =
                                                                        TaggedItem(
                                                                            AppAlertState.General(
                                                                                title = "QR Scan Error",
                                                                                message = "Unable to scan QR code: $error",
                                                                            ),
                                                                        )
                                                                },
                                                            )
                                                            break
                                                        }
                                                    }
                                                }
                                                .addOnFailureListener(mainExecutor) { e ->
                                                    Log.e("QrCodeImportScreen", "Barcode processing failed", e)
                                                }
                                                .addOnCompleteListener {
                                                    imageProxy.close()
                                                }
                                        } else {
                                            imageProxy.close()
                                        }
                                    }
                                }
                        analysisRef.value = imageAnalysis

                        val cameraSelector = CameraSelector.DEFAULT_BACK_CAMERA

                        try {
                            cameraProvider.unbindAll()
                            cameraProvider.bindToLifecycle(
                                lifecycleOwner,
                                cameraSelector,
                                preview,
                                imageAnalysis,
                            )
                        } catch (e: Exception) {
                            Log.e("QrCodeImportScreen", "Camera binding failed", e)
                        }
                    }, ContextCompat.getMainExecutor(ctx))

                    previewView
                },
                modifier = Modifier.fillMaxSize(),
            )

            // overlay content
            Column(
                modifier =
                    Modifier
                        .fillMaxSize()
                        .padding(16.dp),
                verticalArrangement = Arrangement.SpaceBetween,
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                Spacer(modifier = Modifier.weight(1f))

                Text(
                    text = "Scan Wallet Export QR Code",
                    style = MaterialTheme.typography.titleLarge,
                    fontWeight = FontWeight.SemiBold,
                    color = Color.White,
                )

                Spacer(modifier = Modifier.weight(5f))

                // multi-part progress
                if (totalParts != null && partsLeft != null) {
                    Column(
                        modifier =
                            Modifier
                                .background(Color.Black.copy(alpha = 0.7f))
                                .padding(16.dp),
                        horizontalAlignment = Alignment.CenterHorizontally,
                    ) {
                        Text(
                            text = "Scanned $partsScanned of ${totalParts?.toInt()}",
                            style = MaterialTheme.typography.bodyMedium,
                            fontWeight = FontWeight.Medium,
                            color = Color.White,
                        )

                        Spacer(modifier = Modifier.height(4.dp))

                        Text(
                            text = "${partsLeft?.toInt()} parts left",
                            style = MaterialTheme.typography.labelSmall,
                            fontWeight = FontWeight.Bold,
                            color = Color.White.copy(alpha = 0.7f),
                        )
                    }
                }

                Spacer(modifier = Modifier.weight(1f))
            }
        }
    }

    DisposableEffect(Unit) {
        onDispose {
            analysisRef.value?.clearAnalyzer()

            cameraProviderRef.value?.let { cp ->
                val p = previewRef.value
                val a = analysisRef.value
                if (p != null && a != null) {
                    cp.unbind(p, a)
                }
            }

            executor.shutdown()
            barcodeScanner.close()
        }
    }
}

private fun handleQrCode(
    barcode: Barcode,
    multiQr: MultiQr?,
    onMultiQrUpdate: (MultiQr) -> Unit,
    onTotalPartsUpdate: (UInt) -> Unit,
    onPartsLeftUpdate: (UInt) -> Unit,
    onScanComplete: (String) -> Unit,
    onError: (String) -> Unit,
) {
    try {
        val qrString = barcode.rawValue ?: return

        // try to create or use existing multi-qr
        val currentMultiQr =
            multiQr ?: try {
                val newMultiQr = MultiQr.newFromString(qr = qrString)
                onMultiQrUpdate(newMultiQr)
                onTotalPartsUpdate(newMultiQr.totalParts())
                newMultiQr
            } catch (e: Exception) {
                Log.d("QrCodeImportScreen", "Not a BBQr (single QR): ${e.message}")
                // single QR code (not BBQr)
                onScanComplete(qrString)
                return
            }

        // check if it's a BBQr
        if (!currentMultiQr.isBbqr()) {
            onScanComplete(qrString)
            return
        }

        // add part to BBQr
        val result = currentMultiQr.addPart(qr = qrString)
        onPartsLeftUpdate(result.partsLeft())

        if (result.isComplete()) {
            val finalData = result.finalResult()
            onScanComplete(finalData)
        }
    } catch (e: Exception) {
        onError(e.message ?: "Unknown error")
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun HelpSheet(onDismiss: () -> Unit) {
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
