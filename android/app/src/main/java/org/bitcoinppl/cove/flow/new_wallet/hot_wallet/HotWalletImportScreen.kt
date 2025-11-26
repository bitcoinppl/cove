package org.bitcoinppl.cove.flow.new_wallet.hot_wallet

import android.Manifest
import android.app.Activity
import android.util.Log
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.itemsIndexed
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Nfc
import androidx.compose.material.icons.filled.QrCodeScanner
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.LocalLifecycleOwner
import com.google.accompanist.permissions.ExperimentalPermissionsApi
import com.google.accompanist.permissions.isGranted
import com.google.accompanist.permissions.rememberPermissionState
import com.google.mlkit.vision.barcode.BarcodeScanning
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.common.InputImage
import kotlinx.coroutines.delay
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.ImportWalletManager
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.views.DashDotsIndicator
import org.bitcoinppl.cove.views.ImageButton
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.types.*
import java.util.concurrent.Executors
import androidx.camera.core.Preview as CameraPreview

private const val GROUPS_OF = 12

private enum class AlertState {
    None,
    InvalidWords,
    DuplicateWallet,
    GenericError,
}

@Preview(showBackground = true, backgroundColor = 0xFF0D1B2A)
@Composable
private fun HotWalletImportScreenPreview() {
    val snack = remember { SnackbarHostState() }
    val app = remember { AppManager.getInstance() }
    val manager = remember { ImportWalletManager() }
    HotWalletImportScreen(
        app = app,
        manager = manager,
        numberOfWords = NumberOfBip39Words.TWELVE,
        importType = ImportType.MANUAL,
        snackbarHostState = snack,
    )
}

@OptIn(ExperimentalMaterial3Api::class, ExperimentalPermissionsApi::class)
@Composable
fun HotWalletImportScreen(
    app: AppManager,
    manager: ImportWalletManager,
    numberOfWords: NumberOfBip39Words,
    importType: ImportType,
    snackbarHostState: SnackbarHostState = remember { SnackbarHostState() },
) {
    val wordCount =
        when (numberOfWords) {
            NumberOfBip39Words.TWELVE -> 12
            NumberOfBip39Words.TWENTY_FOUR -> 24
        }

    val numberOfGroups = wordCount / GROUPS_OF
    var enteredWords by remember(numberOfWords) {
        mutableStateOf(List(numberOfGroups) { List(GROUPS_OF) { "" } })
    }

    var alertState by remember { mutableStateOf(AlertState.None) }
    var duplicateWalletId by remember { mutableStateOf<WalletId?>(null) }
    var genericErrorMessage by remember { mutableStateOf("") }
    var focusedField by remember(numberOfWords) { mutableIntStateOf(0) }

    // QR and NFC state
    var showQrScanner by remember { mutableStateOf(false) }
    var showNfcScanner by remember { mutableStateOf(false) }
    var multiQr by remember { mutableStateOf<MultiQr?>(null) }

    // auto-open scanner based on importType (matching iOS behavior)
    LaunchedEffect(importType) {
        when (importType) {
            ImportType.QR -> {
                showQrScanner = true
            }
            ImportType.NFC -> {
                // add small delay like iOS (200ms)
                delay(200)
                showNfcScanner = true
            }
            ImportType.MANUAL -> {
                // focus first field
                focusedField = 0
            }
        }
    }

    fun setWords(words: List<List<String>>) {
        // validate word count (must be 12 or 24)
        val totalWords = words.flatten().size
        if (totalWords != 12 && totalWords != 24) {
            Log.w("HotWalletImport", "Invalid word count: $totalWords")
            genericErrorMessage = "Invalid number of words. Expected 12 or 24 words, got $totalWords"
            alertState = AlertState.GenericError
            return
        }

        // reset scanners
        multiQr = null
        showQrScanner = false
        showNfcScanner = false

        // update words
        enteredWords = words

        // move to last field
        focusedField = totalWords - 1
    }

    fun isAllWordsValid(): Boolean =
        enteredWords
            .flatten()
            .withIndex()
            .all { (idx, word) ->
                word.isNotEmpty() &&
                    Bip39WordSpecificAutocomplete(
                        wordNumber = (idx + 1).toUShort(),
                        numberOfWords = numberOfWords,
                    ).isBip39Word(word)
            }

    fun importWallet() {
        try {
            val walletMetadata = manager.importWallet(enteredWords)
            app.rust.selectWallet(walletMetadata.id)
            app.resetRoute(Route.SelectedWallet(walletMetadata.id))
        } catch (e: ImportWalletException.InvalidWordGroup) {
            Log.d("HotWalletImport", "Invalid word group while importing hot wallet")
            alertState = AlertState.InvalidWords
        } catch (e: ImportWalletException.WalletAlreadyExists) {
            duplicateWalletId = e.v1
            alertState = AlertState.DuplicateWallet
        } catch (e: Exception) {
            Log.e("HotWalletImport", "import error", e)
            genericErrorMessage = e.message ?: "Unknown error occurred"
            alertState = AlertState.GenericError
        }
    }

    Scaffold(
        containerColor = CoveColor.midnightBlue,
        topBar = {
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
                        stringResource(R.string.title_import_wallet),
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.SemiBold,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                },
                navigationIcon = {
                    IconButton(onClick = { app.popRoute() }) {
                        Icon(
                            imageVector = Icons.AutoMirrored.Default.ArrowBack,
                            contentDescription = "Back",
                        )
                    }
                },
                actions = {
                    Row {
                        // NFC button
                        IconButton(onClick = { showNfcScanner = true }) {
                            Icon(
                                imageVector = Icons.Default.Nfc,
                                contentDescription = "NFC Import",
                            )
                        }

                        // QR button
                        IconButton(onClick = { showQrScanner = true }) {
                            Icon(
                                imageVector = Icons.Default.QrCodeScanner,
                                contentDescription = "QR Code Import",
                            )
                        }
                    }
                },
            )
        },
        snackbarHost = { SnackbarHost(snackbarHostState) },
    ) { padding ->
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
                        .fillMaxHeight()
                        .align(Alignment.TopCenter),
                alpha = 0.5f,
            )

            Column(
                modifier =
                    Modifier
                        .fillMaxSize()
                        .padding(vertical = 20.dp),
                verticalArrangement = Arrangement.SpaceBetween,
            ) {
                // word input grid
                Column(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 20.dp),
                    verticalArrangement = Arrangement.spacedBy(16.dp),
                ) {
                    Spacer(Modifier.height(24.dp))

                    WordInputGrid(
                        enteredWords = enteredWords,
                        numberOfWords = numberOfWords,
                        focusedField = focusedField,
                        onWordsChanged = { newWords -> enteredWords = newWords },
                        onFocusChanged = { field -> focusedField = field },
                    )
                }

                Spacer(Modifier.weight(1f))

                // bottom section
                Column(
                    verticalArrangement = Arrangement.spacedBy(24.dp),
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 20.dp),
                ) {
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        DashDotsIndicator(
                            count = 5,
                            currentIndex = 2,
                        )
                        Spacer(Modifier.weight(1f))
                    }

                    Text(
                        text = stringResource(R.string.title_import_wallet),
                        color = Color.White,
                        fontSize = 38.sp,
                        fontWeight = FontWeight.SemiBold,
                        lineHeight = 42.sp,
                    )

                    Text(
                        text = stringResource(R.string.label_import_wallet_instructions),
                        color = CoveColor.coveLightGray,
                        fontSize = 15.sp,
                        lineHeight = 20.sp,
                        modifier = Modifier.fillMaxWidth(),
                    )

                    HorizontalDivider(
                        color = CoveColor.coveLightGray.copy(alpha = 0.50f),
                        thickness = 1.dp,
                    )

                    ImageButton(
                        text = stringResource(R.string.action_import_wallet),
                        onClick = { if (isAllWordsValid()) importWallet() },
                        colors =
                            ButtonDefaults.buttonColors(
                                containerColor = CoveColor.btnPrimary,
                                contentColor = CoveColor.midnightBlue,
                                disabledContainerColor = CoveColor.btnPrimary.copy(alpha = 0.5f),
                                disabledContentColor = CoveColor.midnightBlue.copy(alpha = 0.5f),
                            ),
                        modifier = Modifier.fillMaxWidth(),
                    )
                }
            }
        }

        // alerts
        when (alertState) {
            AlertState.InvalidWords -> {
                AlertDialog(
                    onDismissRequest = { alertState = AlertState.None },
                    title = { Text(stringResource(R.string.alert_title_words_not_valid)) },
                    text = { Text(stringResource(R.string.alert_message_words_not_valid)) },
                    confirmButton = {
                        TextButton(onClick = { alertState = AlertState.None }) {
                            Text(stringResource(R.string.btn_ok))
                        }
                    },
                )
            }
            AlertState.DuplicateWallet -> {
                AlertDialog(
                    onDismissRequest = { alertState = AlertState.None },
                    title = { Text(stringResource(R.string.alert_title_duplicate_wallet)) },
                    text = { Text(stringResource(R.string.alert_message_duplicate_wallet)) },
                    confirmButton = {
                        TextButton(
                            onClick = {
                                alertState = AlertState.None
                                duplicateWalletId?.let { walletId ->
                                    app.rust.selectWallet(walletId)
                                    app.resetRoute(Route.SelectedWallet(walletId))
                                }
                            },
                        ) {
                            Text(stringResource(R.string.btn_ok))
                        }
                    },
                )
            }
            AlertState.GenericError -> {
                AlertDialog(
                    onDismissRequest = { alertState = AlertState.None },
                    title = { Text(stringResource(R.string.alert_title_import_error)) },
                    text = { Text(genericErrorMessage) },
                    confirmButton = {
                        TextButton(onClick = { alertState = AlertState.None }) {
                            Text(stringResource(R.string.btn_ok))
                        }
                    },
                )
            }
            AlertState.None -> {}
        }

        // QR Scanner Bottom Sheet
        if (showQrScanner) {
            QrScannerSheet(
                numberOfWords = numberOfWords,
                onDismiss = {
                    showQrScanner = false
                    multiQr = null
                },
                onWordsScanned = { words ->
                    setWords(words)
                },
            )
        }

        // NFC Scanner Bottom Sheet
        if (showNfcScanner) {
            NfcScannerSheet(
                numberOfWords = numberOfWords,
                onDismiss = {
                    showNfcScanner = false
                },
                onWordsScanned = { words ->
                    setWords(words)
                },
            )
        }
    }
}

@Composable
private fun WordInputGrid(
    enteredWords: List<List<String>>,
    numberOfWords: NumberOfBip39Words,
    focusedField: Int,
    onWordsChanged: (List<List<String>>) -> Unit,
    onFocusChanged: (Int) -> Unit,
) {
    val wordCount =
        when (numberOfWords) {
            NumberOfBip39Words.TWELVE -> 12
            NumberOfBip39Words.TWENTY_FOUR -> 24
        }

    val flatWords = enteredWords.flatten()

    LazyVerticalGrid(
        columns = GridCells.Fixed(2),
        horizontalArrangement = Arrangement.spacedBy(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
        modifier = Modifier.fillMaxWidth(),
    ) {
        itemsIndexed(
            items = flatWords,
            key = { index, _ -> "word-input-$index" },
        ) { index, word ->
            WordInputField(
                number = index + 1,
                word = word,
                numberOfWords = numberOfWords,
                allEnteredWords = enteredWords,
                isFocused = focusedField == index,
                onWordChanged = { newWord ->
                    val groupIndex = index / GROUPS_OF
                    val wordIndex = index % GROUPS_OF
                    val newWords = enteredWords.toMutableList()
                    val newGroup = newWords[groupIndex].toMutableList()
                    newGroup[wordIndex] = newWord
                    newWords[groupIndex] = newGroup
                    onWordsChanged(newWords)
                },
                onFocusChanged = { hasFocus ->
                    if (hasFocus) onFocusChanged(index)
                },
                onNext = {
                    if (index < wordCount - 1) {
                        onFocusChanged(index + 1)
                    }
                },
            )
        }
    }
}

@Composable
private fun WordInputField(
    number: Int,
    word: String,
    numberOfWords: NumberOfBip39Words,
    allEnteredWords: List<List<String>>,
    isFocused: Boolean,
    onWordChanged: (String) -> Unit,
    onFocusChanged: (Boolean) -> Unit,
    onNext: () -> Unit,
) {
    val autocomplete =
        remember(number) {
            Bip39WordSpecificAutocomplete(
                wordNumber = number.toUShort(),
                numberOfWords = numberOfWords,
            )
        }

    var suggestions by remember { mutableStateOf<List<String>>(emptyList()) }

    val isValid = word.isNotEmpty() && autocomplete.isBip39Word(word)
    val hasInput = word.isNotEmpty()

    val borderColor =
        when {
            !hasInput -> Color.Transparent
            isValid -> CoveColor.SuccessGreen.copy(alpha = 0.6f)
            else -> CoveColor.ErrorRed.copy(alpha = 0.7f)
        }

    val textColor =
        when {
            !hasInput -> CoveColor.coveLightGray.copy(alpha = 0.45f)
            isValid -> CoveColor.SuccessGreen.copy(alpha = 0.8f)
            else -> CoveColor.ErrorRed
        }

    val focusRequester = remember { FocusRequester() }
    val focusManager = LocalFocusManager.current

    LaunchedEffect(isFocused) {
        if (isFocused) {
            delay(100)
            focusRequester.requestFocus()
        }
    }

    // update suggestions when word or focus changes
    LaunchedEffect(word, isFocused) {
        suggestions =
            if (isFocused && word.isNotEmpty()) {
                autocomplete.autocomplete(word, allEnteredWords)
            } else {
                emptyList()
            }
    }

    Column {
        Row(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .height(40.dp)
                    .background(
                        color = Color.White.copy(alpha = 0.08f),
                        shape = RoundedCornerShape(8.dp),
                    ).border(
                        width = if (borderColor != Color.Transparent) 2.dp else 0.dp,
                        color = borderColor,
                        shape = RoundedCornerShape(8.dp),
                    ).padding(horizontal = 12.dp, vertical = 8.dp),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                text = "$number.",
                color = CoveColor.coveLightGray.copy(alpha = 0.6f),
                fontSize = 12.sp,
                fontWeight = FontWeight.Medium,
                modifier = Modifier.width(28.dp),
            )

            BasicTextField(
                value = word,
                onValueChange = { newValue ->
                    val trimmed = newValue.trim().lowercase()
                    onWordChanged(trimmed)

                    // auto-advance when word is complete and valid
                    if (trimmed.isNotEmpty() && autocomplete.isBip39Word(trimmed)) {
                        suggestions = emptyList()
                        onNext()
                    }
                },
                textStyle =
                    TextStyle(
                        color = textColor,
                        fontSize = 14.sp,
                        fontWeight = FontWeight.Medium,
                        textAlign = TextAlign.End,
                    ),
                singleLine = true,
                cursorBrush = SolidColor(Color.White),
                keyboardOptions =
                    KeyboardOptions(
                        capitalization = KeyboardCapitalization.None,
                        autoCorrectEnabled = false,
                        keyboardType = KeyboardType.Ascii,
                        imeAction = ImeAction.Next,
                    ),
                keyboardActions =
                    KeyboardActions(
                        onNext = { onNext() },
                    ),
                modifier =
                    Modifier
                        .weight(1f)
                        .focusRequester(focusRequester)
                        .onFocusChanged { focusState ->
                            onFocusChanged(focusState.isFocused)
                            if (!focusState.isFocused) {
                                suggestions = emptyList()
                            }
                        },
            )
        }

        // suggestion dropdown
        if (suggestions.isNotEmpty()) {
            Column(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .background(
                            color = CoveColor.midnightBlue.copy(alpha = 0.95f),
                            shape = RoundedCornerShape(bottomStart = 8.dp, bottomEnd = 8.dp),
                        ).border(
                            width = 1.dp,
                            color = CoveColor.coveLightGray.copy(alpha = 0.3f),
                            shape = RoundedCornerShape(bottomStart = 8.dp, bottomEnd = 8.dp),
                        ),
            ) {
                suggestions.forEach { suggestion ->
                    Text(
                        text = suggestion,
                        color = Color.White,
                        fontSize = 14.sp,
                        fontWeight = FontWeight.Medium,
                        modifier =
                            Modifier
                                .fillMaxWidth()
                                .clickable {
                                    onWordChanged(suggestion)
                                    suggestions = emptyList()
                                    onNext()
                                }.padding(horizontal = 12.dp, vertical = 10.dp),
                    )
                }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class, ExperimentalPermissionsApi::class)
@Composable
@androidx.camera.core.ExperimentalGetImage
private fun QrScannerSheet(
    numberOfWords: NumberOfBip39Words,
    onDismiss: () -> Unit,
    onWordsScanned: (List<List<String>>) -> Unit,
) {
    val cameraPermissionState = rememberPermissionState(Manifest.permission.CAMERA)

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        containerColor = Color.Black,
    ) {
        if (cameraPermissionState.status.isGranted) {
            QrScannerContent(
                numberOfWords = numberOfWords,
                onDismiss = onDismiss,
                onWordsScanned = onWordsScanned,
                modifier = Modifier.fillMaxSize(),
            )
        } else {
            // camera permission request
            Column(
                modifier = Modifier.fillMaxWidth().padding(24.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(16.dp),
            ) {
                Text(
                    text = "Camera Access Required",
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.Bold,
                    color = Color.White,
                )

                Text(
                    text = "Please allow camera access to scan QR codes",
                    style = MaterialTheme.typography.bodyMedium,
                    color = Color.White.copy(alpha = 0.7f),
                    textAlign = TextAlign.Center,
                )

                TextButton(onClick = { cameraPermissionState.launchPermissionRequest() }) {
                    Text("Grant Permission", color = Color.White)
                }

                TextButton(onClick = onDismiss) {
                    Text("Cancel", color = Color.White.copy(alpha = 0.6f))
                }
            }
        }
    }
}

@Composable
@androidx.camera.core.ExperimentalGetImage
private fun QrScannerContent(
    numberOfWords: NumberOfBip39Words,
    onDismiss: () -> Unit,
    onWordsScanned: (List<List<String>>) -> Unit,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val lifecycleOwner = LocalLifecycleOwner.current

    var multiQr by remember { mutableStateOf<MultiQr?>(null) }
    var scanComplete by remember { mutableStateOf(false) }
    var totalParts by remember { mutableStateOf<UInt?>(null) }
    var partsLeft by remember { mutableStateOf<UInt?>(null) }

    val partsScanned =
        remember(totalParts, partsLeft) {
            totalParts?.let { total ->
                partsLeft?.let { left ->
                    (total - left).toInt()
                }
            }
        }

    val barcodeScanner = remember { BarcodeScanning.getClient() }
    val executor = remember { Executors.newSingleThreadExecutor() }
    val cameraProviderRef = remember { mutableStateOf<ProcessCameraProvider?>(null) }
    val previewRef = remember { mutableStateOf<CameraPreview?>(null) }
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
                            CameraPreview.Builder().build().also {
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
                                                            handleQrCodeForSeed(
                                                                barcode = barcode,
                                                                numberOfWords = numberOfWords,
                                                                multiQr = multiQr,
                                                                onMultiQrUpdate = { multiQr = it },
                                                                onTotalPartsUpdate = { totalParts = it },
                                                                onPartsLeftUpdate = { partsLeft = it },
                                                                onScanComplete = { words ->
                                                                    scanComplete = true
                                                                    onWordsScanned(words)
                                                                },
                                                                onError = { error ->
                                                                    Log.e("QrScannerSheet", "Error: $error")
                                                                    onDismiss()
                                                                },
                                                            )
                                                            break
                                                        }
                                                    }
                                                }.addOnFailureListener(mainExecutor) { e ->
                                                    Log.e("QrScannerSheet", "Barcode processing failed", e)
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
                            cameraProvider.bindToLifecycle(
                                lifecycleOwner,
                                cameraSelector,
                                preview,
                                imageAnalysis,
                            )
                        } catch (e: Exception) {
                            Log.e("QrScannerSheet", "Camera binding failed", e)
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
                    text = "Scan Seed Phrase QR Code",
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

private fun handleQrCodeForSeed(
    barcode: Barcode,
    numberOfWords: NumberOfBip39Words,
    multiQr: MultiQr?,
    onMultiQrUpdate: (MultiQr) -> Unit,
    onTotalPartsUpdate: (UInt) -> Unit,
    onPartsLeftUpdate: (UInt) -> Unit,
    onScanComplete: (List<List<String>>) -> Unit,
    onError: (String) -> Unit,
) {
    try {
        val qrString = barcode.rawValue ?: return
        val qrBytes = barcode.rawBytes

        // try to parse as MultiQr first (for BBQr/SeedQR)
        val currentMultiQr =
            multiQr ?: try {
                val newMultiQr = MultiQr.newFromString(qr = qrString)
                onMultiQrUpdate(newMultiQr)
                onTotalPartsUpdate(newMultiQr.totalParts())
                newMultiQr
            } catch (e: Exception) {
                Log.d("QrScannerSheet", "Not a BBQr, trying plain text: ${e.message}")
                // try plain text mnemonic
                tryParsePlainTextOrSeedQr(qrString, qrBytes, numberOfWords, onScanComplete, onError)
                return
            }

        // check if it's a BBQr
        if (!currentMultiQr.isBbqr()) {
            tryParsePlainTextOrSeedQr(qrString, qrBytes, numberOfWords, onScanComplete, onError)
            return
        }

        // add part to BBQr
        val result = currentMultiQr.addPart(qr = qrString)
        onPartsLeftUpdate(result.partsLeft())

        if (result.isComplete()) {
            val finalData = result.finalResult()
            tryParsePlainTextOrSeedQr(finalData, null, numberOfWords, onScanComplete, onError)
        }
    } catch (e: Exception) {
        onError(e.message ?: "Unknown error")
    }
}

private fun tryParsePlainTextOrSeedQr(
    qrString: String,
    qrBytes: ByteArray?,
    numberOfWords: NumberOfBip39Words,
    onScanComplete: (List<List<String>>) -> Unit,
    onError: (String) -> Unit,
) {
    try {
        // try parsing as plain text mnemonic first
        val words = groupedPlainWordsOf(mnemonic = qrString, groups = GROUPS_OF.toUByte())
        onScanComplete(words)
    } catch (e: Exception) {
        Log.d("QrScannerSheet", "Not plain text, trying SeedQR: ${e.message}")

        // try parsing as SeedQR (binary format)
        qrBytes?.let { bytes ->
            try {
                val seedQr = SeedQr.newFromData(data = bytes)
                val words = seedQr.groupedPlainWords(groupsOf = GROUPS_OF.toUByte())
                onScanComplete(words)
                return
            } catch (e2: Exception) {
                Log.d("QrScannerSheet", "Not SeedQR binary: ${e2.message}")
            }
        }

        onError("Unable to parse QR code as seed phrase")
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun NfcScannerSheet(
    numberOfWords: NumberOfBip39Words,
    onDismiss: () -> Unit,
    onWordsScanned: (List<List<String>>) -> Unit,
) {
    val context = LocalContext.current
    val activity = context as? Activity

    if (activity == null) {
        // fallback if not in activity context
        ModalBottomSheet(
            onDismissRequest = onDismiss,
            containerColor = CoveColor.midnightBlue,
        ) {
            Column(
                modifier = Modifier.fillMaxWidth().padding(24.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                Text(
                    "Unable to access NFC",
                    style = MaterialTheme.typography.titleMedium,
                    color = Color.White,
                )
                TextButton(onClick = onDismiss) {
                    Text("Close", color = Color.White)
                }
            }
        }
        return
    }

    val nfcReader =
        remember(activity) {
            org.bitcoinppl.cove.nfc
                .NfcReader(activity)
        }
    var errorMessage by remember { mutableStateOf<String?>(null) }

    // start scanning when sheet opens
    LaunchedEffect(Unit) {
        nfcReader.startScanning()

        // listen for scan results
        nfcReader.scanResults.collect { result ->
            when (result) {
                is org.bitcoinppl.cove.nfc.NfcScanResult.Success -> {
                    // try to parse the NFC data as seed words
                    try {
                        // try string format first
                        result.text?.let { text ->
                            val words = groupedPlainWordsOf(mnemonic = text, groups = GROUPS_OF.toUByte())
                            onWordsScanned(words)
                            return@collect
                        }

                        // try binary format (SeedQR)
                        result.data?.let { data ->
                            val seedQr = SeedQr.newFromData(data = data)
                            val words = seedQr.groupedPlainWords(groupsOf = GROUPS_OF.toUByte())
                            onWordsScanned(words)
                            return@collect
                        }

                        errorMessage = "No readable seed phrase found on NFC tag"
                    } catch (e: Exception) {
                        Log.e("NfcScannerSheet", "Error parsing NFC data", e)
                        errorMessage = "Unable to parse seed phrase: ${e.message}"
                    }
                }
                is org.bitcoinppl.cove.nfc.NfcScanResult.Error -> {
                    errorMessage = result.message
                }
            }
        }
    }

    DisposableEffect(Unit) {
        onDispose {
            nfcReader.reset()
        }
    }

    ModalBottomSheet(
        onDismissRequest = {
            nfcReader.reset()
            onDismiss()
        },
        containerColor = CoveColor.midnightBlue,
    ) {
        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            if (nfcReader.isScanning) {
                CircularProgressIndicator(
                    color = Color.White,
                    modifier = Modifier.padding(16.dp),
                )

                Icon(
                    imageVector = Icons.Default.Nfc,
                    contentDescription = null,
                    tint = Color.White,
                    modifier = Modifier.padding(16.dp),
                )

                Text(
                    text = "Ready to Scan",
                    style = MaterialTheme.typography.titleLarge,
                    fontWeight = FontWeight.Bold,
                    color = Color.White,
                )

                Text(
                    text = nfcReader.message,
                    style = MaterialTheme.typography.bodyMedium,
                    color = Color.White.copy(alpha = 0.7f),
                    textAlign = TextAlign.Center,
                )
            } else {
                // show icon and error message when not scanning
                Icon(
                    imageVector = Icons.Default.Nfc,
                    contentDescription = null,
                    tint = Color.White,
                    modifier = Modifier.padding(16.dp),
                )

                Text(
                    text = "NFC Unavailable",
                    style = MaterialTheme.typography.titleLarge,
                    fontWeight = FontWeight.Bold,
                    color = Color.White,
                )
            }

            // show error message regardless of scanning state
            if (errorMessage != null) {
                Spacer(modifier = Modifier.height(8.dp))
                Text(
                    text = errorMessage!!,
                    style = MaterialTheme.typography.bodySmall,
                    color = CoveColor.ErrorRed,
                    textAlign = TextAlign.Center,
                )
            }

            Spacer(modifier = Modifier.height(8.dp))

            TextButton(
                onClick = {
                    nfcReader.reset()
                    onDismiss()
                },
            ) {
                Text("Cancel", color = Color.White)
            }

            Spacer(modifier = Modifier.height(24.dp))
        }
    }
}
