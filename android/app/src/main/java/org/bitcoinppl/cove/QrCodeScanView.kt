package org.bitcoinppl.cove

import android.Manifest
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
import androidx.compose.ui.platform.LocalConfiguration
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
import org.bitcoinppl.cove_core.AppAlertState
import org.bitcoinppl.cove_core.MultiFormat
import org.bitcoinppl.cove_core.QrScanner
import org.bitcoinppl.cove_core.ScanProgress
import org.bitcoinppl.cove_core.ScanResult
import org.bitcoinppl.cove_core.StringOrData
import java.util.concurrent.Executors

@OptIn(ExperimentalPermissionsApi::class)
@Composable
fun QrCodeScanView(
    onScanned: (MultiFormat) -> Unit,
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
    onScanned: (MultiFormat) -> Unit,
    onDismiss: () -> Unit,
    app: AppManager,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val lifecycleOwner = LocalLifecycleOwner.current

    // Rust QR scanner state machine
    val scanner = remember { QrScanner() }
    var isDisposed by remember { mutableStateOf(false) }
    var progress by remember { mutableStateOf<ScanProgress?>(null) }
    var scanComplete by remember { mutableStateOf(false) }
    var scannedData by remember { mutableStateOf<MultiFormat?>(null) }

    // handle scan completion
    LaunchedEffect(scannedData) {
        scannedData?.let { data ->
            onScanned(data)
            scannedData = null
            scanComplete = false
            progress = null
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
                                                    if (scanComplete || isDisposed) return@addOnSuccessListener

                                                    for (barcode in barcodes) {
                                                        if (barcode.format == Barcode.FORMAT_QR_CODE) {
                                                            handleQrCode(
                                                                context = ctx,
                                                                barcode = barcode,
                                                                scanner = scanner,
                                                                onProgress = { progress = it },
                                                                onComplete = { data ->
                                                                    scanComplete = true
                                                                    scannedData = data
                                                                    if (!isDisposed) scanner.reset()
                                                                },
                                                                onError = { error ->
                                                                    if (!isDisposed) scanner.reset()
                                                                    onDismiss()
                                                                    app.alertState =
                                                                        TaggedItem(
                                                                            AppAlertState.General(
                                                                                title = "QR Scan Error",
                                                                                message = error,
                                                                            ),
                                                                        )
                                                                },
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

            // viewfinder overlay - centered (65% of smaller screen dimension, capped 200-320dp)
            val configuration = LocalConfiguration.current
            val screenWidth = configuration.screenWidthDp.dp
            val screenHeight = configuration.screenHeightDp.dp
            val smallerDimension = minOf(screenWidth, screenHeight)
            val viewfinderSize = (smallerDimension * 0.65f).coerceIn(200.dp, 320.dp)

            Canvas(
                modifier =
                    Modifier
                        .size(viewfinderSize)
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

                // multi-part progress (uses displayText/detailText from Rust)
                progress?.let { prog ->
                    Column(
                        modifier =
                            Modifier
                                .background(Color.Black.copy(alpha = 0.7f))
                                .padding(16.dp),
                        horizontalAlignment = Alignment.CenterHorizontally,
                    ) {
                        Text(
                            text = prog.displayText(),
                            style = MaterialTheme.typography.bodyMedium,
                            fontWeight = FontWeight.Medium,
                            color = Color.White,
                        )

                        prog.detailText()?.let { detail ->
                            Spacer(modifier = Modifier.height(4.dp))

                            Text(
                                text = detail,
                                style = MaterialTheme.typography.labelSmall,
                                fontWeight = FontWeight.Bold,
                                color = Color.White.copy(alpha = 0.7f),
                            )
                        }
                    }
                }

                Spacer(modifier = Modifier.weight(1f))
            }
        }
    }

    DisposableEffect(Unit) {
        onDispose {
            isDisposed = true
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
            scanner.close()
        }
    }
}

private fun handleQrCode(
    context: android.content.Context,
    barcode: Barcode,
    scanner: QrScanner,
    onProgress: (ScanProgress) -> Unit,
    onComplete: (MultiFormat) -> Unit,
    onError: (String) -> Unit,
) {
    try {
        // convert barcode to StringOrData (prioritize text, fall back to binary)
        val qrData =
            when {
                barcode.rawValue != null -> StringOrData.String(barcode.rawValue!!)
                barcode.rawBytes != null -> StringOrData.Data(barcode.rawBytes!!)
                else -> return // no data available
            }

        // use Rust state machine to process the QR code
        when (val result = scanner.scan(qrData)) {
            is ScanResult.Complete -> {
                result.haptic.trigger(context)
                onComplete(result.data)
            }
            is ScanResult.InProgress -> {
                result.haptic.trigger(context)
                onProgress(result.progress)
            }
        }
    } catch (e: IllegalStateException) {
        // scanner was destroyed during async callback, ignore
        Log.d("QrCodeScanView", "Scanner already destroyed, ignoring: ${e.message}")
    } catch (e: Exception) {
        onError("Unable to scan QR code: ${e.message ?: "Unknown scanning error"}")
    }
}
