package org.bitcoinppl.cove

import android.Manifest
import android.content.Context
import android.os.Build
import android.os.VibrationEffect
import android.os.Vibrator
import android.os.VibratorManager
import android.util.Base64
import android.util.Log
import androidx.camera.core.Camera
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.FlashOff
import androidx.compose.material.icons.filled.FlashOn
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
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

// haptic feedback helper
private fun triggerHapticFeedback(context: Context) {
    try {
        val vibrator =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                val vibratorManager = context.getSystemService(Context.VIBRATOR_MANAGER_SERVICE) as? VibratorManager
                vibratorManager?.defaultVibrator
            } else {
                @Suppress("DEPRECATION")
                context.getSystemService(Context.VIBRATOR_SERVICE) as? Vibrator
            }

        vibrator?.let {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                it.vibrate(VibrationEffect.createOneShot(50, VibrationEffect.DEFAULT_AMPLITUDE))
            } else {
                @Suppress("DEPRECATION")
                it.vibrate(50)
            }
        }
    } catch (e: Exception) {
        Log.w("QrCodeScanView", "Failed to trigger haptic feedback", e)
    }
}

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

    data class Complete(
        val data: StringOrData,
    ) : QrCodeScannerState()
}

@OptIn(ExperimentalPermissionsApi::class)
@Composable
fun QrCodeScanView(
    onScanned: (StringOrData) -> Unit,
    onDismiss: () -> Unit,
    app: AppManager,
    modifier: Modifier = Modifier,
    showTopBar: Boolean = true,
) {
    val cameraPermissionState = rememberPermissionState(Manifest.permission.CAMERA)

    Box(
        modifier = modifier.fillMaxSize().background(Color.Black),
    ) {
        when {
            cameraPermissionState.status.isGranted -> {
                QrScannerContent(
                    onScanned = onScanned,
                    onDismiss = onDismiss,
                    app = app,
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

        // overlaid back button
        if (showTopBar) {
            IconButton(
                onClick = onDismiss,
                modifier =
                    Modifier
                        .align(Alignment.TopStart)
                        .statusBarsPadding()
                        .padding(16.dp),
            ) {
                Icon(
                    Icons.AutoMirrored.Filled.ArrowBack,
                    contentDescription = "Back",
                    tint = Color.White,
                )
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
    app: AppManager,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val lifecycleOwner = LocalLifecycleOwner.current

    var scannerState by remember { mutableStateOf<QrCodeScannerState>(QrCodeScannerState.Idle) }
    val scannedCodes = remember { mutableSetOf<String>() }

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
    val cameraRef = remember { mutableStateOf<Camera?>(null) }

    // flashlight and zoom state
    var isFlashOn by remember { mutableStateOf(false) }
    var zoomLevel by remember { mutableStateOf(1.2f) } // 1.2 = "1x", 2.0 = 2x

    // toggle flashlight
    fun toggleFlash() {
        cameraRef.value?.let { camera ->
            isFlashOn = !isFlashOn
            camera.cameraControl.enableTorch(isFlashOn)
        }
    }

    // toggle zoom between 1x (1.2f) and 2x
    fun toggleZoom() {
        cameraRef.value?.let { camera ->
            zoomLevel = if (zoomLevel == 1.2f) 2.0f else 1.2f
            camera.cameraControl.setZoomRatio(zoomLevel)
        }
    }

    Box(modifier = modifier) {
        when (val state = scannerState) {
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
                                ImageAnalysis
                                    .Builder()
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
                                                barcodeScanner
                                                    .process(image)
                                                    .addOnSuccessListener(mainExecutor) { barcodes ->
                                                        for (barcode in barcodes) {
                                                            if (barcode.format == Barcode.FORMAT_QR_CODE) {
                                                                handleQrCode(
                                                                    context = ctx,
                                                                    currentState = scannerState,
                                                                    barcode = barcode,
                                                                    scannedCodes = scannedCodes,
                                                                    onStateUpdate = { scannerState = it },
                                                                    onDismiss = onDismiss,
                                                                    app = app,
                                                                )
                                                                break
                                                            }
                                                        }
                                                    }.addOnFailureListener(mainExecutor) { e ->
                                                        Log.e("QrCodeScanView", "Barcode processing failed", e)
                                                    }.addOnCompleteListener {
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
                                val camera =
                                    cameraProvider.bindToLifecycle(
                                        lifecycleOwner,
                                        cameraSelector,
                                        preview,
                                        imageAnalysis,
                                    )
                                cameraRef.value = camera
                                camera.cameraControl.setZoomRatio(zoomLevel)
                            } catch (e: Exception) {
                                Log.e("QrCodeScanView", "Camera binding failed", e)
                                onDismiss()
                                app.alertState =
                                    TaggedItem(
                                        AppAlertState.General(
                                            title = "QR Scan Error",
                                            message = "Failed to initialize camera: ${e.message ?: "Unknown error"}",
                                        ),
                                    )
                            }
                        }, ContextCompat.getMainExecutor(ctx))

                        previewView
                    },
                    modifier = Modifier.fillMaxSize(),
                )

                // flashlight and zoom controls - top of screen
                Row(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .align(Alignment.TopCenter)
                            .statusBarsPadding()
                            .padding(horizontal = 16.dp, vertical = 60.dp),
                    horizontalArrangement = Arrangement.SpaceBetween,
                ) {
                    // flashlight toggle - top left
                    Box(
                        modifier =
                            Modifier
                                .size(44.dp)
                                .background(Color.Black.copy(alpha = 0.5f), CircleShape)
                                .clickable { toggleFlash() },
                        contentAlignment = Alignment.Center,
                    ) {
                        Icon(
                            imageVector = if (isFlashOn) Icons.Filled.FlashOn else Icons.Filled.FlashOff,
                            contentDescription = if (isFlashOn) "Turn off flashlight" else "Turn on flashlight",
                            tint = Color.White,
                            modifier = Modifier.size(24.dp),
                        )
                    }

                    // zoom toggle - top right
                    Box(
                        modifier =
                            Modifier
                                .size(44.dp)
                                .background(Color.Black.copy(alpha = 0.5f), CircleShape)
                                .clickable { toggleZoom() },
                        contentAlignment = Alignment.Center,
                    ) {
                        Text(
                            text = if (zoomLevel == 1.2f) "1x" else "2x",
                            color = Color.White,
                            fontWeight = FontWeight.Medium,
                        )
                    }
                }

                // viewfinder overlay - centered
                Canvas(
                    modifier =
                        Modifier
                            .size(200.dp)
                            .align(Alignment.Center),
                ) {
                    val strokeWidth = 4.dp.toPx()
                    val cornerLength = 40.dp.toPx()
                    val color = Color.White.copy(alpha = 0.7f)

                    // top-left corner
                    drawLine(color, Offset(0f, cornerLength), Offset(0f, 0f), strokeWidth)
                    drawLine(color, Offset(0f, 0f), Offset(cornerLength, 0f), strokeWidth)

                    // top-right corner
                    drawLine(color, Offset(size.width - cornerLength, 0f), Offset(size.width, 0f), strokeWidth)
                    drawLine(color, Offset(size.width, 0f), Offset(size.width, cornerLength), strokeWidth)

                    // bottom-left corner
                    drawLine(color, Offset(0f, size.height - cornerLength), Offset(0f, size.height), strokeWidth)
                    drawLine(color, Offset(0f, size.height), Offset(cornerLength, size.height), strokeWidth)

                    // bottom-right corner
                    drawLine(color, Offset(size.width - cornerLength, size.height), Offset(size.width, size.height), strokeWidth)
                    drawLine(color, Offset(size.width, size.height - cornerLength), Offset(size.width, size.height), strokeWidth)
                }

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
    context: Context,
    currentState: QrCodeScannerState,
    barcode: Barcode,
    scannedCodes: MutableSet<String>,
    onStateUpdate: (QrCodeScannerState) -> Unit,
    onDismiss: () -> Unit,
    app: AppManager,
) {
    // guard against reprocessing after completion
    if (currentState is QrCodeScannerState.Complete) {
        return
    }

    try {
        // check both rawBytes (binary) and rawValue (text)
        // prioritize rawValue (text) if available, fall back to rawBytes (binary)
        val qrData =
            when {
                barcode.rawValue != null -> StringOrData.String(barcode.rawValue!!)
                barcode.rawBytes != null -> StringOrData.Data(barcode.rawBytes!!)
                else -> return // no data available
            }

        // deduplication: check if this code was already scanned
        val qrDataString =
            when (qrData) {
                is StringOrData.String -> qrData.v1
                is StringOrData.Data -> Base64.encodeToString(qrData.v1, Base64.NO_WRAP)
            }

        if (scannedCodes.contains(qrDataString)) {
            return
        }
        scannedCodes.add(qrDataString)

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
                triggerHapticFeedback(context)
                onStateUpdate(QrCodeScannerState.Complete(qrData))
                return
            }

        // check if it's a BBQr
        if (!multiQr.isBbqr()) {
            triggerHapticFeedback(context)
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
                    return
                }
            }

        // add part to BBQr
        val result = multiQr.addPart(qr = qrString)
        val partsLeft = result.partsLeft()

        // haptic feedback for each BBQr part scanned
        triggerHapticFeedback(context)

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
        onDismiss()
        app.alertState =
            TaggedItem(
                AppAlertState.General(
                    title = "QR Scan Error",
                    message = "Unable to scan QR code, error: ${e.message ?: "Unknown scanning error"}",
                ),
            )
    }
}
