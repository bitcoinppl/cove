package org.bitcoinppl.cove.settings

import androidx.activity.compose.BackHandler
import androidx.biometric.BiometricManager
import androidx.biometric.BiometricPrompt
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.AttachMoney
import androidx.compose.material.icons.filled.Fingerprint
import androidx.compose.material.icons.filled.Hub
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.LockOpen
import androidx.compose.material.icons.filled.Masks
import androidx.compose.material.icons.filled.MoreHoriz
import androidx.compose.material.icons.filled.Palette
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material.icons.filled.Wifi
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity
import org.bitcoinppl.cove.Auth
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.views.MaterialDivider
import org.bitcoinppl.cove.views.MaterialSection
import org.bitcoinppl.cove.views.MaterialSettingsItem
import org.bitcoinppl.cove.views.NumberPadPinView
import org.bitcoinppl.cove.views.SectionHeader
import org.bitcoinppl.cove.views.WalletIcon
import org.bitcoinppl.cove_core.AuthManagerAction
import org.bitcoinppl.cove_core.AuthManagerException
import org.bitcoinppl.cove_core.AuthType
import org.bitcoinppl.cove_core.Database
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.SecurityAlertState
import org.bitcoinppl.cove_core.SecuritySettingsAction
import org.bitcoinppl.cove_core.SecuritySettingsResult
import org.bitcoinppl.cove_core.SecuritySheetState
import org.bitcoinppl.cove_core.SettingsRoute
import org.bitcoinppl.cove_core.WalletMetadata
import org.bitcoinppl.cove_core.WalletSettingsRoute

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsScreen(
    app: org.bitcoinppl.cove.AppManager,
    modifier: Modifier = Modifier,
) {
    // track if network has changed (similar to iOS implementation)
    val networkChanged =
        remember(app.previousSelectedNetwork, app.selectedNetwork) {
            app.previousSelectedNetwork != null && app.selectedNetwork != app.previousSelectedNetwork
        }
    var showNetworkChangeAlert by remember { mutableStateOf(false) }

    // intercept back button when network has changed
    BackHandler(enabled = networkChanged) {
        showNetworkChangeAlert = true
    }

    Scaffold(
        modifier =
            modifier
                .fillMaxSize()
                .padding(WindowInsets.safeDrawing.asPaddingValues()),
        topBar = @Composable {
            TopAppBar(
                title = {
                    Box(
                        modifier = Modifier.fillMaxSize(),
                        contentAlignment = Alignment.Center,
                    ) {
                        Text(
                            style = MaterialTheme.typography.bodyLarge,
                            text = stringResource(R.string.title_settings),
                            textAlign = TextAlign.Center,
                        )
                    }
                },
                navigationIcon = {
                    IconButton(
                        onClick = {
                            if (networkChanged) {
                                showNetworkChangeAlert = true
                            } else {
                                app.popRoute()
                            }
                        },
                    ) {
                        Icon(Icons.AutoMirrored.Default.ArrowBack, contentDescription = "Back")
                    }
                },
                actions = { },
                modifier = Modifier.height(56.dp),
            )
        },
        content = { paddingValues ->
            Column(
                modifier =
                    Modifier
                        .fillMaxSize()
                        .verticalScroll(rememberScrollState())
                        .padding(paddingValues),
            ) {
                SectionHeader(stringResource(R.string.title_settings_general))
                MaterialSection {
                    Column {
                        MaterialSettingsItem(
                            title = stringResource(R.string.title_settings_network),
                            icon = Icons.Default.Wifi,
                            onClick = {
                                app.pushRoute(
                                    org.bitcoinppl.cove_core.Route
                                        .Settings(org.bitcoinppl.cove_core.SettingsRoute.Network),
                                )
                            },
                        )
                        MaterialDivider()
                        MaterialSettingsItem(
                            title = stringResource(R.string.title_settings_appearance),
                            icon = Icons.Default.Palette,
                            onClick = {
                                app.pushRoute(
                                    org.bitcoinppl.cove_core.Route
                                        .Settings(org.bitcoinppl.cove_core.SettingsRoute.Appearance),
                                )
                            },
                        )
                        MaterialDivider()
                        MaterialSettingsItem(
                            title = stringResource(R.string.title_settings_node),
                            icon = Icons.Default.Hub,
                            onClick = {
                                app.pushRoute(
                                    org.bitcoinppl.cove_core.Route
                                        .Settings(org.bitcoinppl.cove_core.SettingsRoute.Node),
                                )
                            },
                        )
                        MaterialDivider()
                        MaterialSettingsItem(
                            title = stringResource(R.string.title_settings_currency),
                            icon = Icons.Default.AttachMoney,
                            onClick = {
                                app.pushRoute(
                                    org.bitcoinppl.cove_core.Route
                                        .Settings(org.bitcoinppl.cove_core.SettingsRoute.FiatCurrency),
                                )
                            },
                        )
                    }
                }

                WalletSettingsSection(app = app)

                SecuritySection(app = app)
            }
        },
    )

    // network change alert
    if (showNetworkChangeAlert) {
        AlertDialog(
            onDismissRequest = { showNetworkChangeAlert = false },
            title = { Text("⚠️ Network Changed ⚠️") },
            text = {
                val networkName =
                    org.bitcoinppl.cove_core.types
                        .networkToString(app.selectedNetwork)
                Text("You've changed your network to $networkName")
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        app.rust.selectLatestOrNewWallet()
                        app.confirmNetworkChange()
                        showNetworkChangeAlert = false
                        app.popRoute()
                    },
                ) {
                    Text("Yes, Change Network")
                }
            },
            dismissButton = {
                TextButton(
                    onClick = { showNetworkChangeAlert = false },
                ) {
                    Text("Cancel")
                }
            },
        )
    }
}

@Composable
private fun WalletSettingsSection(app: org.bitcoinppl.cove.AppManager) {
    var wallets by remember { mutableStateOf<List<WalletMetadata>>(emptyList()) }

    // fetch all wallets on screen appear
    LaunchedEffect(Unit) {
        wallets = Database().wallets().allSortedActive()
    }

    // don't show section if there are no wallets
    if (wallets.isEmpty()) {
        return
    }

    val topAmount = 5
    val top5Wallets = wallets.take(topAmount)
    val hasMore = wallets.size > topAmount

    SectionHeader("Wallet Settings")
    MaterialSection {
        Column {
            top5Wallets.forEachIndexed { index, wallet ->
                MaterialSettingsItem(
                    title = wallet.name,
                    leadingContent = {
                        WalletIcon(wallet = wallet, size = 28.dp, cornerRadius = 6.dp)
                    },
                    onClick = {
                        app.pushRoute(
                            Route.Settings(
                                SettingsRoute.Wallet(
                                    id = wallet.id,
                                    route = WalletSettingsRoute.MAIN,
                                ),
                            ),
                        )
                    },
                )
                if (index < top5Wallets.size - 1 || hasMore) {
                    MaterialDivider()
                }
            }

            if (hasMore) {
                MaterialSettingsItem(
                    title = "More",
                    icon = Icons.Default.MoreHoriz,
                    onClick = {
                        app.pushRoute(
                            Route.Settings(SettingsRoute.AllWallets),
                        )
                    },
                )
            }
        }
    }
}

@Composable
private fun SecuritySection(app: org.bitcoinppl.cove.AppManager) {
    val context = LocalContext.current
    val activity = context as? FragmentActivity
    val auth = Auth
    val biometricManager = remember { BiometricManager.from(context) }

    val isBiometricAvailable =
        remember {
            biometricManager.canAuthenticate(BiometricManager.Authenticators.BIOMETRIC_STRONG) ==
                BiometricManager.BIOMETRIC_SUCCESS
        }

    // sheet and alert state (using Rust enums)
    var sheetState: SecuritySheetState by remember { mutableStateOf(SecuritySheetState.NONE) }
    var alertState: SecurityAlertState? by remember { mutableStateOf(null) }

    // local state for decoy mode (settings changes only affect UI, not persisted)
    var decoyModePinEnabled by remember { mutableStateOf(true) }
    var decoyModeFaceIdEnabled by remember { mutableStateOf(false) }
    var decoyModeWipeDataPinEnabled by remember { mutableStateOf(false) }
    var decoyModeDecoyPinEnabled by remember { mutableStateOf(false) }

    // track which action triggered decoy mode update (for local state updates)
    var lastDecoyAction: SecuritySettingsAction? by remember { mutableStateOf(null) }

    // computed toggle values
    val isBiometricEnabled =
        if (auth.isInDecoyMode()) {
            decoyModeFaceIdEnabled
        } else {
            auth.type == AuthType.BOTH || auth.type == AuthType.BIOMETRIC
        }

    val isPinEnabled =
        if (auth.isInDecoyMode()) {
            decoyModePinEnabled
        } else {
            auth.type == AuthType.BOTH || auth.type == AuthType.PIN
        }

    val isWipeDataPinEnabled =
        if (auth.isInDecoyMode()) {
            decoyModeWipeDataPinEnabled
        } else {
            auth.isWipeDataPinEnabled
        }

    val isDecoyPinEnabled =
        if (auth.isInDecoyMode()) {
            decoyModeDecoyPinEnabled
        } else {
            auth.isDecoyPinEnabled
        }

    // handle security result from Rust validation
    fun handleSecurityResult(result: SecuritySettingsResult, action: SecuritySettingsAction) {
        when (result) {
            is SecuritySettingsResult.ProceedToSheet -> sheetState = result.v1
            is SecuritySettingsResult.ShowAlert -> alertState = result.v1
            is SecuritySettingsResult.DecoyModeLocalUpdate -> {
                lastDecoyAction = action
                when (action) {
                    is SecuritySettingsAction.ToggleBiometric -> decoyModeFaceIdEnabled = action.enable
                    is SecuritySettingsAction.TogglePin -> decoyModePinEnabled = action.enable
                    is SecuritySettingsAction.ToggleWipeDataPin -> decoyModeWipeDataPinEnabled = action.enable
                    is SecuritySettingsAction.ToggleDecoyPin -> decoyModeDecoyPinEnabled = action.enable
                    is SecuritySettingsAction.ChangePin -> {}
                }
            }
        }
    }

    // toggle handlers using Rust validation
    fun onBiometricToggle(enable: Boolean) {
        val action = SecuritySettingsAction.ToggleBiometric(enable)
        val result = auth.rust.validateSecurityAction(action, app.rust.unverifiedWalletIds())
        handleSecurityResult(result, action)
    }

    fun onPinToggle(enable: Boolean) {
        val action = SecuritySettingsAction.TogglePin(enable)
        val result = auth.rust.validateSecurityAction(action, app.rust.unverifiedWalletIds())
        handleSecurityResult(result, action)
    }

    fun onWipeDataPinToggle(enable: Boolean) {
        val action = SecuritySettingsAction.ToggleWipeDataPin(enable)
        val result = auth.rust.validateSecurityAction(action, app.rust.unverifiedWalletIds())
        handleSecurityResult(result, action)
    }

    fun onDecoyPinToggle(enable: Boolean) {
        val action = SecuritySettingsAction.ToggleDecoyPin(enable)
        val result = auth.rust.validateSecurityAction(action, app.rust.unverifiedWalletIds())
        handleSecurityResult(result, action)
    }

    // setter functions
    fun setPin(pin: String) {
        if (auth.isInDecoyMode()) {
            decoyModePinEnabled = true
            sheetState = SecuritySheetState.NONE
            return
        }
        auth.dispatch(AuthManagerAction.SetPin(pin))
        sheetState = SecuritySheetState.NONE
    }

    fun setWipeDataPin(pin: String) {
        sheetState = SecuritySheetState.NONE
        if (auth.isInDecoyMode()) {
            decoyModeWipeDataPinEnabled = true
            return
        }

        try {
            auth.rust.setWipeDataPin(pin)
        } catch (e: AuthManagerException) {
            alertState = SecurityAlertState.ExtraSetPinError(e.message ?: "Unknown error")
        }
    }

    fun setDecoyPin(pin: String) {
        sheetState = SecuritySheetState.NONE
        if (auth.isInDecoyMode()) {
            decoyModeDecoyPinEnabled = true
            return
        }

        try {
            auth.rust.setDecoyPin(pin)
        } catch (e: AuthManagerException) {
            alertState = SecurityAlertState.ExtraSetPinError(e.message ?: "Unknown error")
        }
    }

    // biometric prompt for enabling biometric
    val biometricPrompt =
        remember(activity) {
            if (activity == null) return@remember null

            BiometricPrompt(
                activity,
                ContextCompat.getMainExecutor(context),
                object : BiometricPrompt.AuthenticationCallback() {
                    override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
                        super.onAuthenticationError(errorCode, errString)
                        sheetState = SecuritySheetState.NONE
                    }

                    override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                        super.onAuthenticationSucceeded(result)
                        auth.dispatch(AuthManagerAction.EnableBiometric)
                        sheetState = SecuritySheetState.NONE
                    }

                    override fun onAuthenticationFailed() {
                        super.onAuthenticationFailed()
                    }
                },
            )
        }

    val promptInfo =
        remember {
            BiometricPrompt.PromptInfo
                .Builder()
                .setTitle("Enable Biometric")
                .setSubtitle("Authenticate to enable biometric unlock")
                .setNegativeButtonText("Cancel")
                .build()
        }

    // trigger biometric prompt when entering EnableBiometric state
    LaunchedEffect(sheetState) {
        if (sheetState == SecuritySheetState.ENABLE_BIOMETRIC) {
            biometricPrompt?.authenticate(promptInfo)
        }
    }

    SectionHeader("Security")
    MaterialSection {
        Column {
            var itemCount = 0

            // biometric toggle
            if (isBiometricAvailable) {
                MaterialSettingsItem(
                    title = "Enable Biometric",
                    icon = Icons.Default.Fingerprint,
                    isSwitch = true,
                    switchCheckedState = isBiometricEnabled,
                    onCheckChanged = { enabled -> onBiometricToggle(enabled) },
                )
                itemCount++
            }

            // PIN toggle
            if (itemCount > 0) MaterialDivider()
            MaterialSettingsItem(
                title = "Enable PIN",
                icon = Icons.Default.Lock,
                isSwitch = true,
                switchCheckedState = isPinEnabled,
                onCheckChanged = { enabled -> onPinToggle(enabled) },
            )
            itemCount++

            // show additional PIN options when PIN is enabled
            if (isPinEnabled) {
                // change PIN
                MaterialDivider()
                MaterialSettingsItem(
                    title = "Change PIN",
                    icon = Icons.Default.LockOpen,
                    onClick = { sheetState = SecuritySheetState.CHANGE_PIN },
                )
                itemCount++

                // wipe data PIN toggle
                MaterialDivider()
                MaterialSettingsItem(
                    title = "Enable Wipe Data PIN",
                    icon = Icons.Default.Warning,
                    isSwitch = true,
                    switchCheckedState = isWipeDataPinEnabled,
                    onCheckChanged = { enabled -> onWipeDataPinToggle(enabled) },
                )
                itemCount++

                // decoy PIN toggle
                MaterialDivider()
                MaterialSettingsItem(
                    title = "Enable Decoy PIN",
                    icon = Icons.Default.Masks,
                    isSwitch = true,
                    switchCheckedState = isDecoyPinEnabled,
                    onCheckChanged = { enabled -> onDecoyPinToggle(enabled) },
                )
            }
        }
    }

    // alert dialogs
    alertState?.let { state ->
        SecurityAlertDialog(
            state = state,
            onDismiss = { alertState = null },
            onConfirmSheet = { nextSheet ->
                alertState = null
                sheetState = nextSheet
            },
            onConfirmAlert = { nextAlert ->
                alertState = nextAlert
            },
            auth = auth,
            app = app,
        )
    }

    // full-screen sheet dialogs
    if (sheetState != SecuritySheetState.NONE && sheetState != SecuritySheetState.ENABLE_BIOMETRIC) {
        SecuritySheetDialog(
            state = sheetState,
            onDismiss = { sheetState = SecuritySheetState.NONE },
            onNextState = { nextState -> sheetState = nextState },
            onSetPin = ::setPin,
            onSetWipeDataPin = ::setWipeDataPin,
            onSetDecoyPin = ::setDecoyPin,
            auth = auth,
            onAlertState = { alertState = it },
        )
    }
}

@Composable
private fun SecurityAlertDialog(
    state: SecurityAlertState,
    onDismiss: () -> Unit,
    onConfirmSheet: (SecuritySheetState) -> Unit,
    onConfirmAlert: (SecurityAlertState) -> Unit,
    auth: org.bitcoinppl.cove.AuthManager,
    app: org.bitcoinppl.cove.AppManager,
) {
    when (state) {
        is SecurityAlertState.UnverifiedWallets -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text("Can't Enable Wipe Data PIN") },
                text = {
                    Text(
                        "You have wallets that have not been backed up. Please back up your wallets before " +
                            "enabling the Wipe Data PIN. If you wipe the data without having a backup of your " +
                            "wallet, you will lose the bitcoin in that wallet.",
                    )
                },
                confirmButton = {
                    TextButton(
                        onClick = {
                            try {
                                app.rust.selectWallet(state.walletId)
                            } catch (_: Exception) {
                            }
                            onDismiss()
                        },
                    ) {
                        Text("Go To Wallet")
                    }
                },
                dismissButton = {
                    TextButton(onClick = onDismiss) {
                        Text("Cancel")
                    }
                },
            )
        }

        SecurityAlertState.ConfirmEnableWipeMePin -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text("Are you sure?") },
                text = {
                    Text(
                        "Enabling the Wipe Data PIN will let you choose a PIN that if entered will wipe all " +
                            "Cove wallet data on this device.\n\nIf you wipe the data without having a backup " +
                            "of your wallet, you will lose the bitcoin in that wallet.\n\nPlease make sure you " +
                            "have a backup of your wallet before enabling this.",
                    )
                },
                confirmButton = {
                    TextButton(onClick = { onConfirmSheet(SecuritySheetState.ENABLE_WIPE_DATA_PIN) }) {
                        Text("Yes, Enable Wipe Data PIN")
                    }
                },
                dismissButton = {
                    TextButton(onClick = onDismiss) {
                        Text("Cancel")
                    }
                },
            )
        }

        SecurityAlertState.ConfirmDecoyPin -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text("Are you sure?") },
                text = {
                    Text(
                        "Enabling Decoy PIN will let you choose a PIN that if entered, will show you a different " +
                            "set of wallets.\n\nThese wallets will only be accessible by entering the decoy PIN " +
                            "instead of your regular PIN.\n\nTo access your regular wallets, you will have to close " +
                            "the app, start it again and enter your regular PIN.",
                    )
                },
                confirmButton = {
                    TextButton(onClick = { onConfirmSheet(SecuritySheetState.ENABLE_DECOY_PIN) }) {
                        Text("Yes, Enable Decoy PIN")
                    }
                },
                dismissButton = {
                    TextButton(onClick = onDismiss) {
                        Text("Cancel")
                    }
                },
            )
        }

        SecurityAlertState.NotePinRequired -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text("PIN is required") },
                text = { Text("Setting a PIN is required to have a wipe data PIN or decoy PIN.") },
                confirmButton = {
                    TextButton(onClick = onDismiss) {
                        Text("OK")
                    }
                },
            )
        }

        SecurityAlertState.NoteFaceIdDisablingForWipeMePin -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text("Disable Biometric Unlock?") },
                text = {
                    Text(
                        "Enabling this trick PIN will disable biometric unlock for Cove.\n\nGoing forward, you " +
                            "will have to use your PIN to unlock Cove.",
                    )
                },
                confirmButton = {
                    TextButton(
                        onClick = {
                            auth.dispatch(AuthManagerAction.DisableBiometric)
                            onConfirmAlert(SecurityAlertState.ConfirmEnableWipeMePin)
                        },
                    ) {
                        Text("Disable Biometric")
                    }
                },
                dismissButton = {
                    TextButton(onClick = onDismiss) {
                        Text("Cancel")
                    }
                },
            )
        }

        SecurityAlertState.NoteFaceIdDisablingForDecoyPin -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text("Disable Biometric Unlock?") },
                text = {
                    Text(
                        "Enabling this trick PIN will disable biometric unlock for Cove.\n\nGoing forward, you " +
                            "will have to use your PIN to unlock Cove.",
                    )
                },
                confirmButton = {
                    TextButton(
                        onClick = {
                            auth.dispatch(AuthManagerAction.DisableBiometric)
                            onConfirmAlert(SecurityAlertState.ConfirmDecoyPin)
                        },
                    ) {
                        Text("Disable Biometric")
                    }
                },
                dismissButton = {
                    TextButton(onClick = onDismiss) {
                        Text("Cancel")
                    }
                },
            )
        }

        SecurityAlertState.NoteNoFaceIdWhenTrickPins -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text("Can't do that") },
                text = {
                    Text(
                        "You can't have Decoy PIN & Wipe Data Pin enabled and biometric active at the same time.\n\n" +
                            "Do you want to disable both of these trick PINs and enable biometric?",
                    )
                },
                confirmButton = {
                    TextButton(onClick = { onConfirmSheet(SecuritySheetState.REMOVE_ALL_TRICK_PINS) }) {
                        Text("Yes, Disable trick PINs")
                    }
                },
                dismissButton = {
                    TextButton(onClick = onDismiss) {
                        Text("Cancel")
                    }
                },
            )
        }

        SecurityAlertState.NoteNoFaceIdWhenWipeMePin -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text("Can't do that") },
                text = { Text("You can't have both Wipe Data PIN and biometric active at the same time.") },
                confirmButton = {
                    TextButton(
                        onClick = {
                            // if no decoy PIN, we can enable biometric after removing wipe data PIN
                            val nextSheet =
                                if (!auth.isDecoyPinEnabled) {
                                    SecuritySheetState.REMOVE_WIPE_DATA_PIN_THEN_ENABLE_BIOMETRIC
                                } else {
                                    SecuritySheetState.REMOVE_WIPE_DATA_PIN
                                }
                            onConfirmSheet(nextSheet)
                        },
                    ) {
                        Text("Disable Wipe Data PIN")
                    }
                },
                dismissButton = {
                    TextButton(onClick = onDismiss) {
                        Text("Cancel")
                    }
                },
            )
        }

        SecurityAlertState.NoteNoFaceIdWhenDecoyPin -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text("Can't do that") },
                text = { Text("You can't have both Decoy PIN and biometric active at the same time.") },
                confirmButton = {
                    TextButton(
                        onClick = {
                            // if no wipe data PIN, we can enable biometric after removing decoy PIN
                            val nextSheet =
                                if (!auth.isWipeDataPinEnabled) {
                                    SecuritySheetState.REMOVE_DECOY_PIN_THEN_ENABLE_BIOMETRIC
                                } else {
                                    SecuritySheetState.REMOVE_DECOY_PIN
                                }
                            onConfirmSheet(nextSheet)
                        },
                    ) {
                        Text("Disable Decoy PIN")
                    }
                },
                dismissButton = {
                    TextButton(onClick = onDismiss) {
                        Text("Cancel")
                    }
                },
            )
        }

        is SecurityAlertState.ExtraSetPinError -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text("Something went wrong!") },
                text = { Text(state.message) },
                confirmButton = {
                    TextButton(onClick = onDismiss) {
                        Text("OK")
                    }
                },
            )
        }
    }
}

@Composable
private fun SecuritySheetDialog(
    state: SecuritySheetState,
    onDismiss: () -> Unit,
    onNextState: (SecuritySheetState) -> Unit,
    onSetPin: (String) -> Unit,
    onSetWipeDataPin: (String) -> Unit,
    onSetDecoyPin: (String) -> Unit,
    auth: org.bitcoinppl.cove.AuthManager,
    onAlertState: (SecurityAlertState) -> Unit,
) {
    Dialog(
        onDismissRequest = onDismiss,
        properties = DialogProperties(usePlatformDefaultWidth = false),
    ) {
        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .background(Color.Black),
        ) {
            when (state) {
                SecuritySheetState.NEW_PIN -> {
                    NewPinView(
                        onComplete = onSetPin,
                        backAction = onDismiss,
                    )
                }

                SecuritySheetState.REMOVE_PIN -> {
                    NumberPadPinView(
                        title = "Enter Current PIN",
                        isPinCorrect = { pin ->
                            if (auth.isInDecoyMode()) auth.checkDecoyPin(pin) else auth.checkPin(pin)
                        },
                        showPin = false,
                        backAction = onDismiss,
                        onUnlock = {
                            if (auth.isInDecoyMode()) {
                                onDismiss()
                                return@NumberPadPinView
                            }
                            auth.dispatch(AuthManagerAction.DisablePin)
                            auth.dispatch(AuthManagerAction.DisableWipeDataPin)
                            onDismiss()
                        },
                    )
                }

                SecuritySheetState.CHANGE_PIN -> {
                    ChangePinView(
                        isPinCorrect = { pin ->
                            if (auth.isInDecoyMode()) auth.checkDecoyPin(pin) else auth.checkPin(pin)
                        },
                        backAction = onDismiss,
                        onComplete = { pin ->
                            if (auth.isInDecoyMode()) {
                                onDismiss()
                                return@ChangePinView
                            }

                            // use Rust validation for new PIN
                            val error = auth.rust.validateNewPin(pin)
                            if (error != null) {
                                onDismiss()
                                onAlertState(SecurityAlertState.ExtraSetPinError(error))
                                return@ChangePinView
                            }

                            onSetPin(pin)
                        },
                    )
                }

                SecuritySheetState.DISABLE_BIOMETRIC -> {
                    NumberPadPinView(
                        title = "Enter Current PIN",
                        isPinCorrect = { pin -> auth.checkPin(pin) },
                        showPin = false,
                        backAction = onDismiss,
                        onUnlock = {
                            auth.dispatch(AuthManagerAction.DisableBiometric)
                            onDismiss()
                        },
                    )
                }

                SecuritySheetState.REMOVE_WIPE_DATA_PIN -> {
                    NumberPadPinView(
                        title = "Enter Current PIN",
                        isPinCorrect = { pin -> auth.checkPin(pin) },
                        showPin = false,
                        backAction = onDismiss,
                        onUnlock = {
                            if (auth.isInDecoyMode()) {
                                onDismiss()
                                return@NumberPadPinView
                            }
                            auth.dispatch(AuthManagerAction.DisableWipeDataPin)
                            onDismiss()
                        },
                    )
                }

                SecuritySheetState.REMOVE_WIPE_DATA_PIN_THEN_ENABLE_BIOMETRIC -> {
                    NumberPadPinView(
                        title = "Enter Current PIN",
                        isPinCorrect = { pin -> auth.checkPin(pin) },
                        showPin = false,
                        backAction = onDismiss,
                        onUnlock = {
                            auth.dispatch(AuthManagerAction.DisableWipeDataPin)
                            onNextState(SecuritySheetState.ENABLE_BIOMETRIC)
                        },
                    )
                }

                SecuritySheetState.REMOVE_DECOY_PIN -> {
                    NumberPadPinView(
                        title = "Enter Current PIN",
                        isPinCorrect = { pin -> auth.checkPin(pin) },
                        showPin = false,
                        backAction = onDismiss,
                        onUnlock = {
                            auth.dispatch(AuthManagerAction.DisableDecoyPin)
                            onDismiss()
                        },
                    )
                }

                SecuritySheetState.REMOVE_DECOY_PIN_THEN_ENABLE_BIOMETRIC -> {
                    NumberPadPinView(
                        title = "Enter Current PIN",
                        isPinCorrect = { pin -> auth.checkPin(pin) },
                        showPin = false,
                        backAction = onDismiss,
                        onUnlock = {
                            auth.dispatch(AuthManagerAction.DisableDecoyPin)
                            onNextState(SecuritySheetState.ENABLE_BIOMETRIC)
                        },
                    )
                }

                SecuritySheetState.REMOVE_ALL_TRICK_PINS -> {
                    NumberPadPinView(
                        title = "Enter Current PIN",
                        isPinCorrect = { pin -> auth.checkPin(pin) },
                        showPin = false,
                        backAction = onDismiss,
                        onUnlock = {
                            auth.dispatch(AuthManagerAction.DisableDecoyPin)
                            auth.dispatch(AuthManagerAction.DisableWipeDataPin)
                            onNextState(SecuritySheetState.ENABLE_BIOMETRIC)
                        },
                    )
                }

                SecuritySheetState.ENABLE_WIPE_DATA_PIN -> {
                    WipeDataPinView(
                        onComplete = onSetWipeDataPin,
                        backAction = onDismiss,
                    )
                }

                SecuritySheetState.ENABLE_DECOY_PIN -> {
                    DecoyPinView(
                        onComplete = onSetDecoyPin,
                        backAction = onDismiss,
                    )
                }

                else -> {}
            }
        }
    }
}
