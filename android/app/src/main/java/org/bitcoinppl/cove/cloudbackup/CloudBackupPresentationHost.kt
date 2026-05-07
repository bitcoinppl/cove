package org.bitcoinppl.cove.cloudbackup

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.AuthManager
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupEnableContext
import org.bitcoinppl.cove_core.CloudBackupPasskeyHint
import org.bitcoinppl.cove_core.CloudBackupPasskeyChoiceIntent
import org.bitcoinppl.cove_core.CloudBackupPromptIntent
import org.bitcoinppl.cove_core.CloudBackupVerificationSource
import org.bitcoinppl.cove_core.DeepVerificationFailure
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.SettingsRoute
import org.bitcoinppl.cove_core.VerificationState

val LocalCloudBackupPresentationCoordinator =
    compositionLocalOf<CloudBackupPresentationCoordinator?> { null }

internal sealed class CloudBackupRootPresentation {
    data class ExistingBackupFound(
        val context: CloudBackupEnableContext,
        val passkeyHint: CloudBackupPasskeyHint?,
    ) : CloudBackupRootPresentation()

    data class PasskeyChoice(
        val intent: CloudBackupPasskeyChoiceIntent,
    ) : CloudBackupRootPresentation()

    data object MissingPasskeyReminder : CloudBackupRootPresentation()

    data object VerificationPrompt : CloudBackupRootPresentation()
}

data class CloudBackupPresentationContext(
    val isActivityResumed: Boolean = false,
    val isUnlocked: Boolean = false,
    val isInDecoyMode: Boolean = false,
    val isCoverPresented: Boolean = true,
    val appHasAlert: Boolean = false,
    val appHasSheet: Boolean = false,
    val isViewingCloudBackup: Boolean = false,
    val presentationPolicy: CloudBackupPresentationPolicy = CloudBackupPresentationPolicy.REQUIRES_UNLOCKED_AUTH,
)

enum class CloudBackupPresentationPolicy {
    REQUIRES_UNLOCKED_AUTH,
    ONBOARDING,
}

private val CloudBackupPresentationPolicy.requiresUnlockedAuth: Boolean
    get() = this == CloudBackupPresentationPolicy.REQUIRES_UNLOCKED_AUTH

private val CloudBackupPresentationPolicy.suppressesGenericPrompts: Boolean
    get() = this == CloudBackupPresentationPolicy.ONBOARDING

enum class CloudBackupPresentationBlocker {
    SETTINGS_LOCAL_MODAL,
    CLOUD_BACKUP_DETAIL_DIALOG,
}

internal fun isCloudBackupPresentationPresentable(
    presentation: CloudBackupRootPresentation,
    context: CloudBackupPresentationContext,
    hasBlockers: Boolean,
): Boolean {
    if (!context.isActivityResumed) return false
    if (context.presentationPolicy.requiresUnlockedAuth && !context.isUnlocked) return false
    if (context.isInDecoyMode) return false
    if (context.isCoverPresented) return false
    if (context.appHasAlert) return false
    if (context.appHasSheet) return false
    if (hasBlockers) return false

    return when (presentation) {
        is CloudBackupRootPresentation.ExistingBackupFound,
        is CloudBackupRootPresentation.PasskeyChoice,
        -> true
        CloudBackupRootPresentation.MissingPasskeyReminder,
        CloudBackupRootPresentation.VerificationPrompt,
        -> !context.presentationPolicy.suppressesGenericPrompts && !context.isViewingCloudBackup
    }
}

@Stable
class CloudBackupPresentationCoordinator {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var transitionJob: Job? = null
    private var ignoreNextDismissEvent = false
    private var requiresPresentationDelay = false
    private var context = CloudBackupPresentationContext()
    private val blockers = mutableSetOf<CloudBackupPresentationBlocker>()

    internal var currentPresentation by mutableStateOf<CloudBackupRootPresentation?>(null)

    private var queuedPresentation: CloudBackupRootPresentation? = null

    fun update(context: CloudBackupPresentationContext) {
        this.context = context
        reconcile()
    }

    fun setBlocker(
        blocker: CloudBackupPresentationBlocker,
        active: Boolean,
    ) {
        if (active) {
            blockers += blocker
        } else {
            blockers -= blocker
        }
        reconcile()
    }

    fun dismissCurrentPresentation() {
        transitionJob?.cancel()
        transitionJob = null
        queuedPresentation = null
        if (currentPresentation != null) {
            requiresPresentationDelay = true
            ignoreNextDismissEvent = true
            currentPresentation = null
        }
    }

    fun consumeDismissEvent(): Boolean =
        if (ignoreNextDismissEvent) {
            ignoreNextDismissEvent = false
            true
        } else {
            false
        }

    fun reconcile() {
        val desired = CloudBackupManager.getInstance().promptIntent.toRootPresentation()

        if (desired == null) {
            requiresPresentationDelay = false
            clearVisiblePresentation()
            return
        }

        if (!isPresentable(desired)) {
            transitionJob?.cancel()
            transitionJob = null
            queuedPresentation = desired
            if (currentPresentation != null) {
                ignoreNextDismissEvent = true
                currentPresentation = null
            }
            return
        }

        if (currentPresentation == desired) {
            transitionJob?.cancel()
            transitionJob = null
            queuedPresentation = null
            requiresPresentationDelay = false
            ignoreNextDismissEvent = false
            return
        }

        if (currentPresentation == null) {
            transitionJob?.cancel()
            transitionJob = null
            if (requiresPresentationDelay) {
                queuedPresentation = desired
                scheduleQueuedPresentation()
            } else {
                queuedPresentation = null
                ignoreNextDismissEvent = false
                currentPresentation = desired
            }
            return
        }

        queuedPresentation = desired
        requiresPresentationDelay = true
        ignoreNextDismissEvent = true
        currentPresentation = null
        scheduleQueuedPresentation()
    }

    private fun clearVisiblePresentation() {
        transitionJob?.cancel()
        transitionJob = null
        queuedPresentation = null
        if (currentPresentation != null) {
            requiresPresentationDelay = true
            ignoreNextDismissEvent = true
            currentPresentation = null
        }
    }

    private fun scheduleQueuedPresentation() {
        transitionJob?.cancel()
        transitionJob =
            scope.launch {
                delay(PRESENTATION_DELAY_MS)
                transitionJob = null
                val queued = queuedPresentation ?: return@launch
                if (CloudBackupManager.getInstance().promptIntent.toRootPresentation() != queued) {
                    queuedPresentation = null
                    return@launch
                }
                if (!isPresentable(queued)) {
                    return@launch
                }
                requiresPresentationDelay = false
                ignoreNextDismissEvent = false
                currentPresentation = queued
                queuedPresentation = null
            }
    }

    fun dispose() {
        transitionJob?.cancel()
        transitionJob = null
        scope.cancel()
    }

    private fun isPresentable(presentation: CloudBackupRootPresentation): Boolean {
        return isCloudBackupPresentationPresentable(
            presentation = presentation,
            context = context,
            hasBlockers = blockers.isNotEmpty(),
        )
    }

    companion object {
        private const val PRESENTATION_DELAY_MS = 800L
    }
}

private fun CloudBackupPromptIntent.toRootPresentation(): CloudBackupRootPresentation? =
    when (this) {
        is CloudBackupPromptIntent.None -> null
        is CloudBackupPromptIntent.ExistingBackupFound -> CloudBackupRootPresentation.ExistingBackupFound(v1, v2)
        is CloudBackupPromptIntent.PasskeyChoice -> CloudBackupRootPresentation.PasskeyChoice(v1)
        is CloudBackupPromptIntent.MissingPasskeyReminder -> CloudBackupRootPresentation.MissingPasskeyReminder
        is CloudBackupPromptIntent.VerificationPrompt -> CloudBackupRootPresentation.VerificationPrompt
    }

private fun existingPasskeyButtonTitle(hint: CloudBackupPasskeyHint?): String =
    hint?.let { "Use Existing Passkey (${it.nameSuffix})" } ?: "Use Existing Passkey"

private fun existingBackupMessage(hint: CloudBackupPasskeyHint?): String =
    hint?.let {
        "Creating a new Cloud Backup will not include wallets from your previous backup. If you still have access to the passkey named Cove Cloud Backup (${it.nameSuffix}), use that passkey instead."
    } ?: "Creating a new Cloud Backup will not include wallets from your previous backup. If you still have access to the passkey for that backup, use the existing passkey instead."

@Composable
fun CloudBackupPresentationHost(
    app: AppManager,
    auth: AuthManager,
    isCoverPresented: Boolean,
    presentationPolicy: CloudBackupPresentationPolicy = CloudBackupPresentationPolicy.REQUIRES_UNLOCKED_AUTH,
    content: @Composable () -> Unit,
) {
    val manager = remember { CloudBackupManager.getInstance() }
    val coordinator = remember { CloudBackupPresentationCoordinator() }
    val lifecycleOwner = LocalLifecycleOwner.current
    var isActivityResumed by remember { mutableStateOf(lifecycleOwner.lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED)) }

    val context =
        CloudBackupPresentationContext(
            isActivityResumed = isActivityResumed,
            isUnlocked = !auth.isLocked,
            isInDecoyMode = auth.isInDecoyMode(),
            isCoverPresented = isCoverPresented,
            appHasAlert = app.alertState != null,
            appHasSheet = app.sheetState != null,
            isViewingCloudBackup = app.currentRoute == Route.Settings(SettingsRoute.CloudBackup),
            presentationPolicy = presentationPolicy,
        )

    DisposableEffect(lifecycleOwner) {
        val observer =
            LifecycleEventObserver { _, event ->
                isActivityResumed =
                    when (event) {
                        Lifecycle.Event.ON_RESUME -> true
                        Lifecycle.Event.ON_PAUSE -> false
                        else -> isActivityResumed
                    }
            }
        lifecycleOwner.lifecycle.addObserver(observer)
        onDispose {
            lifecycleOwner.lifecycle.removeObserver(observer)
        }
    }

    DisposableEffect(coordinator) {
        onDispose {
            coordinator.dispose()
        }
    }

    LaunchedEffect(context) {
        coordinator.update(context)
    }

    LaunchedEffect(manager.promptIntent) {
        coordinator.reconcile()
    }

    androidx.compose.runtime.CompositionLocalProvider(
        LocalCloudBackupPresentationCoordinator provides coordinator,
    ) {
        content()
    }

    when (val presentation = coordinator.currentPresentation) {
        is CloudBackupRootPresentation.ExistingBackupFound -> {
            AlertDialog(
                onDismissRequest = {
                    if (!coordinator.consumeDismissEvent()) {
                        manager.dispatch(CloudBackupManagerAction.DiscardPendingEnableCloudBackup)
                    }
                },
                title = { Text("Existing Cloud Backup Found") },
                text = {
                    Text(existingBackupMessage(presentation.passkeyHint))
                },
                confirmButton = {
                    TextButton(
                        onClick = {
                            coordinator.dismissCurrentPresentation()
                            manager.dispatch(
                                enableCloudBackupForceNew(presentation.context),
                            )
                        },
                    ) { Text("Create New Backup") }
                },
                dismissButton = {
                    TextButton(
                        onClick = {
                            coordinator.dismissCurrentPresentation()
                            manager.dispatch(CloudBackupManagerAction.DiscardPendingEnableCloudBackup)
                        },
                    ) { Text("Cancel") }
                },
            )
        }

        is CloudBackupRootPresentation.PasskeyChoice -> {
            AlertDialog(
                onDismissRequest = {
                    if (!coordinator.consumeDismissEvent()) {
                        manager.dispatch(CloudBackupManagerAction.DismissPasskeyChoicePrompt)
                    }
                },
                title = { Text("Passkey Options") },
                text = {
                    Text("Would you like to use an existing passkey or create a new one?")
                },
                confirmButton = {
                    Column(horizontalAlignment = Alignment.End) {
                        TextButton(
                            onClick = {
                                coordinator.dismissCurrentPresentation()
                                when (val intent = presentation.intent) {
                                    is CloudBackupPasskeyChoiceIntent.Enable ->
                                        manager.dispatch(
                                            CloudBackupManagerAction.EnableCloudBackup(intent.v1),
                                        )
                                    is CloudBackupPasskeyChoiceIntent.RepairPasskey ->
                                        manager.dispatch(CloudBackupManagerAction.RepairPasskey)
                                }
                            },
                        ) {
                            Text(
                                existingPasskeyButtonTitle(
                                    (presentation.intent as? CloudBackupPasskeyChoiceIntent.Enable)?.v2,
                                ),
                            )
                        }
                        TextButton(
                            onClick = {
                                coordinator.dismissCurrentPresentation()
                                when (val intent = presentation.intent) {
                                    is CloudBackupPasskeyChoiceIntent.Enable ->
                                        manager.dispatch(
                                            CloudBackupManagerAction.EnableCloudBackupNoDiscovery(intent.v1),
                                        )
                                    is CloudBackupPasskeyChoiceIntent.RepairPasskey ->
                                        manager.dispatch(CloudBackupManagerAction.RepairPasskeyNoDiscovery)
                                }
                            },
                        ) { Text("Create New Passkey") }
                        TextButton(
                            onClick = {
                                coordinator.dismissCurrentPresentation()
                                manager.dispatch(CloudBackupManagerAction.DismissPasskeyChoicePrompt)
                            },
                        ) { Text("Cancel") }
                    }
                },
            )
        }

        CloudBackupRootPresentation.MissingPasskeyReminder -> {
            AlertDialog(
                onDismissRequest = {
                    if (!coordinator.consumeDismissEvent()) {
                        manager.dispatch(CloudBackupManagerAction.DismissMissingPasskeyReminder)
                    }
                },
                title = { Text("Cloud Backup Passkey Missing") },
                text = {
                    Text(
                        "Add a new passkey to restore access to your cloud backup. Until you do, your backups can't be restored.",
                    )
                },
                confirmButton = {
                    TextButton(
                        onClick = {
                            coordinator.dismissCurrentPresentation()
                            if (app.currentRoute != Route.Settings(SettingsRoute.CloudBackup)) {
                                app.pushRoute(Route.Settings(SettingsRoute.CloudBackup))
                            }
                        },
                    ) { Text("Open Cloud Backup") }
                },
                dismissButton = {
                    TextButton(
                        onClick = {
                            coordinator.dismissCurrentPresentation()
                            manager.dispatch(CloudBackupManagerAction.DismissMissingPasskeyReminder)
                        },
                    ) { Text("Not Now") }
                },
            )
        }

        CloudBackupRootPresentation.VerificationPrompt -> {
            CloudBackupVerificationPrompt(
                manager = manager,
                onDismiss = {
                    coordinator.dismissCurrentPresentation()
                    manager.dispatch(CloudBackupManagerAction.DismissVerificationPrompt)
                },
                onVerify = {
                    manager.dispatch(
                        CloudBackupManagerAction.StartVerification(
                            CloudBackupVerificationSource.ROOT_PROMPT,
                        ),
                    )
                },
            )
        }

        null -> Unit
    }
}

@Composable
private fun CloudBackupVerificationPrompt(
    manager: CloudBackupManager,
    onDismiss: () -> Unit,
    onVerify: () -> Unit,
) {
    val isVerifying = manager.verification is VerificationState.Verifying
    val failure =
        if (manager.shouldPromptVerification) {
            null
        } else {
            (manager.verification as? VerificationState.Failed)?.v1
        }

    val title =
        when {
            isVerifying -> "Verifying Cloud Backup"
            failure != null -> "Verification Failed"
            else -> "Verify"
        }

    val message =
        when {
            failure != null -> failure.message()
            isVerifying ->
                "Confirming your updated cloud backup can be decrypted and restored. Continuing may ask for your passkey."
            else ->
                "Verify your updated cloud backup now to confirm it is accessible. Continuing may ask for your passkey."
        }

    Dialog(
        onDismissRequest = {
            if (!isVerifying) {
                onDismiss()
            }
        },
        properties =
            DialogProperties(
                dismissOnBackPress = !isVerifying,
                dismissOnClickOutside = false,
                usePlatformDefaultWidth = false,
            ),
    ) {
        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .background(Color(0xE6000000)),
        ) {
            Surface(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .align(Alignment.Center)
                        .padding(24.dp),
                shape = MaterialTheme.shapes.extraLarge,
                tonalElevation = 6.dp,
            ) {
                Column(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .padding(24.dp),
                    verticalArrangement = Arrangement.spacedBy(16.dp),
                ) {
                    Box(modifier = Modifier.fillMaxWidth()) {
                        if (!isVerifying) {
                            IconButton(
                                onClick = onDismiss,
                                modifier = Modifier.align(Alignment.TopEnd),
                            ) {
                                Icon(Icons.Default.Close, contentDescription = "Close")
                            }
                        }
                    }

                    Icon(
                        imageVector = if (failure == null) Icons.Default.CheckCircle else Icons.Default.Warning,
                        contentDescription = null,
                        tint = if (failure == null) Color(0xFF2E7D32) else Color(0xFFED6C02),
                        modifier = Modifier.align(Alignment.CenterHorizontally),
                    )

                    Text(title, style = MaterialTheme.typography.headlineSmall)
                    Text(message, style = MaterialTheme.typography.bodyMedium)

                    Button(
                        onClick = onVerify,
                        enabled = !isVerifying,
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        if (isVerifying) {
                            CircularProgressIndicator(
                                strokeWidth = 2.dp,
                                modifier = Modifier.padding(end = 8.dp).height(18.dp),
                            )
                        }
                        Text(if (failure == null) "Verify" else "Try Again")
                    }

                    if (!isVerifying) {
                        TextButton(
                            onClick = onDismiss,
                            modifier = Modifier.fillMaxWidth(),
                        ) {
                            Text("Not Now")
                        }
                    }
                }
            }
        }
    }
}
