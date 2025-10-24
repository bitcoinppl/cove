package org.bitcoinppl.cove.sheets

import android.Manifest
import android.util.Log
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
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
import org.bitcoinppl.cove_core.StringOrData
import java.util.concurrent.Executors

/**
 * qr scanner sheet - camera-based QR code scanner with BBQr support
 * ported from iOS QrCodeScanView.swift
 */
@OptIn(ExperimentalPermissionsApi::class, ExperimentalMaterial3Api::class)
@Composable
fun QrScannerSheet(
    app: AppManager,
    onScanned: (StringOrData) -> Unit,
    onDismiss: () -> Unit
) {
    val cameraPermissionState = rememberPermissionState(Manifest.permission.CAMERA)

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        containerColor = Color.Black
    ) {
        when {
            cameraPermissionState.status.isGranted -> {
                QrScannerContent(
                    app = app,
                    onScanned = onScanned,
                    onDismiss = onDismiss
                )
            }
            else -> {
                PermissionDeniedContent(
                    app = app,
                    permissionState = cameraPermissionState,
                    onDismiss = onDismiss
                )
            }
        }
    }
}

@OptIn(ExperimentalPermissionsApi::class)
@Composable
private fun PermissionDeniedContent(
    app: AppManager,
    permissionState: PermissionState,
    onDismiss: () -> Unit
) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .fillMaxHeight(0.5f)
            .padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center
    ) {
        Text(
            text = "Camera Permission Required",
            style = MaterialTheme.typography.titleMedium,
            fontWeight = FontWeight.Bold,
            color = Color.White
        )

        Spacer(modifier = Modifier.height(16.dp))

        Text(
            text = "Please grant camera permission to scan QR codes",
            style = MaterialTheme.typography.bodyMedium,
            color = Color.White.copy(alpha = 0.7f)
        )

        Spacer(modifier = Modifier.height(24.dp))

        Button(
            onClick = {
                permissionState.launchPermissionRequest()
            }
        ) {
            Text("Grant Permission")
        }

        Spacer(modifier = Modifier.height(12.dp))

        TextButton(
            onClick = {
                onDismiss()
                app.alertState = TaggedItem(AppAlertState.NoCameraPermission)
            }
        ) {
            Text("Cancel", color = Color.White)
        }
    }
}

@Composable
@androidx.camera.core.ExperimentalGetImage
private fun QrScannerContent(
    app: AppManager,
    onScanned: (StringOrData) -> Unit,
    onDismiss: () -> Unit
) {
    val context = LocalContext.current
    val lifecycleOwner = LocalLifecycleOwner.current

    var multiQr by remember { mutableStateOf<MultiQr?>(null) }
    var scanComplete by remember { mutableStateOf(false) }
    var totalParts by remember { mutableStateOf<UInt?>(null) }
    var partsLeft by remember { mutableStateOf<UInt?>(null) }

    val partsScanned = remember(totalParts, partsLeft) {
        totalParts?.let { total ->
            partsLeft?.let { left ->
                (total - left).toInt()
            }
        }
    }

    val barcodeScanner = remember { BarcodeScanning.getClient() }
    val executor = remember { Executors.newSingleThreadExecutor() }

    Box(
        modifier = Modifier
            .fillMaxWidth()
            .fillMaxHeight(0.8f)
    ) {
        if (!scanComplete) {
            // camera preview
            AndroidView(
                factory = { ctx ->
                    val previewView = PreviewView(ctx)
                    val cameraProviderFuture = ProcessCameraProvider.getInstance(ctx)

                    cameraProviderFuture.addListener({
                        val cameraProvider = cameraProviderFuture.get()

                        val preview = Preview.Builder().build().also {
                            it.setSurfaceProvider(previewView.surfaceProvider)
                        }

                        val imageAnalysis = ImageAnalysis.Builder()
                            .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                            .build()
                            .also { analysis ->
                                analysis.setAnalyzer(executor) { imageProxy ->
                                    val mediaImage = imageProxy.image
                                    if (mediaImage != null) {
                                        val image = InputImage.fromMediaImage(
                                            mediaImage,
                                            imageProxy.imageInfo.rotationDegrees
                                        )

                                        barcodeScanner.process(image)
                                            .addOnSuccessListener { barcodes ->
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
                                                                onScanned(it)
                                                                onDismiss()
                                                            },
                                                            onError = { error ->
                                                                Log.e("QrScanner", "Error: $error")
                                                                onDismiss()
                                                                app.alertState = TaggedItem(
                                                                    AppAlertState.General(
                                                                        "QR Scan Error",
                                                                        "Unable to scan QR code: $error"
                                                                    )
                                                                )
                                                            }
                                                        )
                                                        break
                                                    }
                                                }
                                            }
                                            .addOnCompleteListener {
                                                imageProxy.close()
                                            }
                                    } else {
                                        imageProxy.close()
                                    }
                                }
                            }

                        val cameraSelector = CameraSelector.DEFAULT_BACK_CAMERA

                        try {
                            cameraProvider.unbindAll()
                            cameraProvider.bindToLifecycle(
                                lifecycleOwner,
                                cameraSelector,
                                preview,
                                imageAnalysis
                            )
                        } catch (e: Exception) {
                            Log.e("QrScanner", "Camera binding failed", e)
                        }
                    }, ContextCompat.getMainExecutor(ctx))

                    previewView
                },
                modifier = Modifier.fillMaxSize()
            )

            // multi-part progress overlay
            if (totalParts != null && partsLeft != null) {
                Column(
                    modifier = Modifier
                        .align(Alignment.BottomCenter)
                        .padding(bottom = 120.dp)
                        .background(Color.Black.copy(alpha = 0.7f))
                        .padding(16.dp),
                    horizontalAlignment = Alignment.CenterHorizontally
                ) {
                    Text(
                        text = "Scanned $partsScanned of ${totalParts?.toInt()} parts",
                        style = MaterialTheme.typography.bodyMedium,
                        fontWeight = FontWeight.Medium,
                        color = Color.White
                    )

                    Spacer(modifier = Modifier.height(4.dp))

                    Text(
                        text = "${partsLeft?.toInt()} parts left",
                        style = MaterialTheme.typography.labelSmall,
                        fontWeight = FontWeight.Bold,
                        color = Color.White.copy(alpha = 0.7f)
                    )
                }
            }
        }
    }

    DisposableEffect(Unit) {
        onDispose {
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
    onScanComplete: (StringOrData) -> Unit,
    onError: (String) -> Unit
) {
    try {
        val qrData = barcode.rawValue ?: return
        val stringOrData = StringOrData.String(qrData)

        // try to create or use existing multi-qr
        val currentMultiQr = multiQr ?: try {
            val newMultiQr = MultiQr.tryNew(qr = stringOrData)
            onMultiQrUpdate(newMultiQr)
            onTotalPartsUpdate(newMultiQr.totalParts())
            newMultiQr
        } catch (e: Exception) {
            // single QR code (not BBQr)
            onScanComplete(stringOrData)
            return
        }

        // check if it's a BBQr
        if (!currentMultiQr.isBbqr()) {
            onScanComplete(stringOrData)
            return
        }

        // add part to BBQr
        val result = currentMultiQr.addPart(qr = StringOrData.String(qrData))
        onPartsLeftUpdate(result.partsLeft())

        if (result.isComplete()) {
            val finalData = result.finalResult()
            onScanComplete(finalData)
        }
    } catch (e: Exception) {
        onError(e.message ?: "Unknown error")
    }
}
