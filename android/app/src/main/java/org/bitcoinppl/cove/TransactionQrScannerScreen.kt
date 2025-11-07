package org.bitcoinppl.cove

import android.Manifest
import android.util.Log
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
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
import kotlinx.coroutines.launch
import org.bitcoinppl.cove_core.BitcoinTransaction
import org.bitcoinppl.cove_core.Database
import org.bitcoinppl.cove_core.MultiQr
import org.bitcoinppl.cove_core.RouteFactory
import java.util.concurrent.Executors

@OptIn(ExperimentalPermissionsApi::class, ExperimentalMaterial3Api::class)
@Composable
fun TransactionQrScannerScreen(
    app: AppManager,
    onDismiss: () -> Unit = {},
    modifier: Modifier = Modifier,
) {
    val cameraPermissionState = rememberPermissionState(Manifest.permission.CAMERA)

    Scaffold(
        topBar = {
            TopAppBar(
                title = { },
                navigationIcon = {
                    IconButton(onClick = { onDismiss() }) {
                        Icon(
                            Icons.AutoMirrored.Filled.ArrowBack,
                            contentDescription = "Back",
                            tint = Color.White,
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
                        onDismiss = onDismiss,
                        modifier = Modifier.fillMaxSize(),
                    )
                }
                else -> {
                    PermissionDeniedContent(
                        app = app,
                        permissionState = cameraPermissionState,
                        onDismiss = onDismiss,
                        modifier = Modifier.fillMaxSize(),
                    )
                }
            }
        }
    }
}

@OptIn(ExperimentalPermissionsApi::class)
@Composable
private fun PermissionDeniedContent(
    app: AppManager,
    permissionState: PermissionState,
    onDismiss: () -> Unit,
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
            text = "Please allow camera access to scan signed transactions.",
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
                onDismiss()
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
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val lifecycleOwner = LocalLifecycleOwner.current
    val scope = rememberCoroutineScope()

    var multiQr by remember { mutableStateOf<MultiQr?>(null) }
    var scanComplete by remember { mutableStateOf(false) }
    var totalParts by remember { mutableStateOf<UInt?>(null) }
    var partsLeft by remember { mutableStateOf<UInt?>(null) }
    var scannedCode by remember { mutableStateOf<String?>(null) }
    var errorMessage by remember { mutableStateOf<String?>(null) }

    val partsScanned =
        remember(totalParts, partsLeft) {
            totalParts?.let { total ->
                partsLeft?.let { left ->
                    (total - left).toInt()
                }
            }
        }

    // handle transaction import when scannedCode changes
    LaunchedEffect(scannedCode) {
        scannedCode?.let { txHex ->
            scope.launch {
                try {
                    val bitcoinTransaction = BitcoinTransaction(txHex = txHex)
                    val db = Database().unsignedTransactions()
                    val txnRecord = db.getTxThrow(txId = bitcoinTransaction.txId())

                    val route =
                        RouteFactory().sendConfirm(
                            id = txnRecord.walletId(),
                            details = txnRecord.confirmDetails(),
                            signedTransaction = bitcoinTransaction,
                        )

                    onDismiss() // dismiss scanner
                    app.pushRoute(route)
                } catch (e: Exception) {
                    Log.e("TransactionQrScanner", "Error importing transaction: $e")
                    errorMessage = e.message ?: "Failed to import signed transaction"
                    onDismiss()
                    app.alertState =
                        TaggedItem(
                            AppAlertState.General(
                                title = "Import Error",
                                message = errorMessage ?: "Failed to import transaction",
                            ),
                        )
                }
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
                                                                    Log.e("TransactionQrScanner", "Error: $error")
                                                                    errorMessage = error
                                                                },
                                                            )
                                                            break
                                                        }
                                                    }
                                                }
                                                .addOnFailureListener(mainExecutor) { e ->
                                                    Log.e("TransactionQrScanner", "Barcode processing failed", e)
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
                            Log.e("TransactionQrScanner", "Camera binding failed", e)
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
                    text = "Scan Signed Transaction",
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
                Log.d("TransactionQrScanner", "Not a BBQr (single QR): ${e.message}")
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
