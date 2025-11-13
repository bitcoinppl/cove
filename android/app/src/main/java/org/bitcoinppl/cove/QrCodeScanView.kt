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
import androidx.compose.material.icons.filled.Error
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.LocalLifecycleOwner
import com.google.accompanist.permissions.ExperimentalPermissionsApi
import com.google.accompanist.permissions.PermissionState
import com.google.accompanist.permissions.isGranted
import com.google.accompanist.permissions.rememberPermissionState
import com.google.mlkit.vision.barcode.BarcodeScanning
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.common.InputImage
import org.bitcoinppl.cove_core.MultiQr
import org.bitcoinppl.cove_core.StringOrData
import java.util.concurrent.Executors

private sealed class QrCodeScannerState {
    object Idle : QrCodeScannerState()

    data class Scanning(
        val multiQr: MultiQr? = null,
        val totalParts: UInt? = null,
        val partsLeft: UInt? = null,
    ) : QrCodeScannerState() {
        val partsScanned: Int?
            get() =
                totalParts?.let { total ->
                    partsLeft?.let { left ->
                        (total - left).toInt()
                    }
                }

        val isMultiPart: Boolean
            get() = totalParts != null && partsLeft != null
    }

    data class Error(val message: String) : QrCodeScannerState()

    data class Complete(val data: StringOrData) : QrCodeScannerState()
}

@OptIn(ExperimentalPermissionsApi::class, ExperimentalMaterial3Api::class)
@Composable
fun QrCodeScanView(
    onScanned: (StringOrData) -> Unit,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val cameraPermissionState = rememberPermissionState(Manifest.permission.CAMERA)

    Scaffold(
        topBar = {
            TopAppBar(
                title = { },
                navigationIcon = {
                    IconButton(onClick = onDismiss) {
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
                        onScanned = onScanned,
                        onDismiss = onDismiss,
                        modifier = Modifier.fillMaxSize(),
                    )
                }
                else -> {
                    PermissionDeniedContent(
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
            text = "Please allow camera access to scan QR codes.",
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
            onClick = onDismiss,
        ) {
            Text("Cancel", color = Color.White)
        }
    }
}

@Composable
@androidx.camera.core.ExperimentalGetImage
private fun QrScannerContent(
    onScanned: (StringOrData) -> Unit,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val lifecycleOwner = LocalLifecycleOwner.current

    var scannerState by remember { mutableStateOf<QrCodeScannerState>(QrCodeScannerState.Idle) }

    // handle scan completion
    LaunchedEffect(scannerState) {
        if (scannerState is QrCodeScannerState.Complete) {
            val data = (scannerState as QrCodeScannerState.Complete).data
            onScanned(data)
            // reset state to prevent re-trigger on recomposition
            scannerState = QrCodeScannerState.Idle
        }
    }

    val barcodeScanner = remember { BarcodeScanning.getClient() }
    val executor = remember { Executors.newSingleThreadExecutor() }
    val cameraProviderRef = remember { mutableStateOf<ProcessCameraProvider?>(null) }
    val previewRef = remember { mutableStateOf<Preview?>(null) }
    val analysisRef = remember { mutableStateOf<ImageAnalysis?>(null) }

    Box(modifier = modifier) {
        when (val state = scannerState) {
            is QrCodeScannerState.Error -> {
                // show error overlay with retry option
                Column(
                    modifier =
                        Modifier
                            .fillMaxSize()
                            .background(Color.Black.copy(alpha = 0.9f))
                            .padding(24.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.Center,
                ) {
                    Icon(
                        imageVector = Icons.Default.Error,
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.error,
                        modifier = Modifier.size(64.dp),
                    )

                    Spacer(modifier = Modifier.height(16.dp))

                    Text(
                        text = "Scan Failed",
                        style = MaterialTheme.typography.titleLarge,
                        fontWeight = FontWeight.Bold,
                        color = Color.White,
                    )

                    Spacer(modifier = Modifier.height(8.dp))

                    Text(
                        text = state.message,
                        style = MaterialTheme.typography.bodyMedium,
                        color = Color.White.copy(alpha = 0.7f),
                        textAlign = TextAlign.Center,
                    )

                    Spacer(modifier = Modifier.height(24.dp))

                    Button(
                        onClick = { scannerState = QrCodeScannerState.Idle },
                    ) {
                        Text("Try Again")
                    }
                }
            }

            is QrCodeScannerState.Complete -> {
                // scanning complete, transitioning to onScanned callback
            }

            else -> {
                // camera preview for Idle and Scanning states
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
                                                                    currentState = scannerState,
                                                                    barcode = barcode,
                                                                    onStateUpdate = { scannerState = it },
                                                                )
                                                                break
                                                            }
                                                        }
                                                    }
                                                    .addOnFailureListener(mainExecutor) { e ->
                                                        Log.e("QrCodeScanView", "Barcode processing failed", e)
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
                                Log.e("QrCodeScanView", "Camera binding failed", e)
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
                        text = "Scan QR Code",
                        style = MaterialTheme.typography.titleLarge,
                        fontWeight = FontWeight.SemiBold,
                        color = Color.White,
                    )

                    Spacer(modifier = Modifier.weight(5f))

                    // multi-part progress
                    if (state is QrCodeScannerState.Scanning && state.isMultiPart) {
                        Column(
                            modifier =
                                Modifier
                                    .background(Color.Black.copy(alpha = 0.7f))
                                    .padding(16.dp),
                            horizontalAlignment = Alignment.CenterHorizontally,
                        ) {
                            Text(
                                text = "Scanned ${state.partsScanned} of ${state.totalParts?.toInt()}",
                                style = MaterialTheme.typography.bodyMedium,
                                fontWeight = FontWeight.Medium,
                                color = Color.White,
                            )

                            Spacer(modifier = Modifier.height(4.dp))

                            Text(
                                text = "${state.partsLeft?.toInt()} parts left",
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
    currentState: QrCodeScannerState,
    barcode: Barcode,
    onStateUpdate: (QrCodeScannerState) -> Unit,
) {
    try {
        // check both rawBytes (binary) and rawValue (text)
        // prioritize rawValue (text) if available, fall back to rawBytes (binary)
        val qrData =
            when {
                barcode.rawValue != null -> StringOrData.String(barcode.rawValue!!)
                barcode.rawBytes != null -> StringOrData.Data(barcode.rawBytes!!)
                else -> return // no data available
            }

        val scanningState =
            when (currentState) {
                is QrCodeScannerState.Scanning -> currentState
                else -> QrCodeScannerState.Scanning()
            }

        // try to create or use existing multi-qr
        val multiQr =
            scanningState.multiQr ?: try {
                val newMultiQr = MultiQr.tryNew(qr = qrData)
                val totalParts = newMultiQr.totalParts()
                onStateUpdate(
                    QrCodeScannerState.Scanning(
                        multiQr = newMultiQr,
                        totalParts = totalParts,
                    ),
                )
                newMultiQr
            } catch (e: Exception) {
                Log.d("QrCodeScanView", "Not a BBQr (single QR): ${e.message}")
                // single QR code (not BBQr)
                onStateUpdate(QrCodeScannerState.Complete(qrData))
                return
            }

        // check if it's a BBQr
        if (!multiQr.isBbqr()) {
            onStateUpdate(QrCodeScannerState.Complete(qrData))
            return
        }

        // for BBQr parts, we need to use the string representation
        // extract the string from StringOrData for addPart
        val qrString =
            when (qrData) {
                is StringOrData.String -> qrData.v1
                is StringOrData.Data -> {
                    // skip binary QR codes for multi-part BBQr, keep scanning
                    Log.d("QrCodeScanView", "Skipping binary QR code in multi-part scan")
                    return
                }
            }

        // add part to BBQr
        val result = multiQr.addPart(qr = qrString)
        val partsLeft = result.partsLeft()

        onStateUpdate(
            scanningState.copy(
                multiQr = multiQr,
                partsLeft = partsLeft,
                totalParts = scanningState.totalParts ?: multiQr.totalParts(),
            ),
        )

        if (result.isComplete()) {
            val finalData = result.finalResult()
            // finalResult returns a string, so wrap it in StringOrData
            onStateUpdate(QrCodeScannerState.Complete(StringOrData.String(finalData)))
        }
    } catch (e: Exception) {
        onStateUpdate(QrCodeScannerState.Error(e.message ?: "Unknown scanning error"))
    }
}
