package org.bitcoinppl.cove.cloudbackup

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
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
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
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
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove.UiText
import org.bitcoinppl.cove.localizedMessage
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove.ui.theme.coveColors
import org.bitcoinppl.cove.views.ChoiceAlertDialog
import org.bitcoinppl.cove.views.DialogChoice
import org.bitcoinppl.cove_core.AppAlertState
import org.bitcoinppl.cove_core.CloudBackupEnablePromptChoice
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupEnableContext
import org.bitcoinppl.cove_core.CloudBackupPasskeyChoiceIntent
import org.bitcoinppl.cove_core.CloudBackupPasskeyHint
import org.bitcoinppl.cove_core.CloudBackupRootPrompt
import org.bitcoinppl.cove_core.CloudBackupVerificationPresentation
import org.bitcoinppl.cove_core.CloudBackupVerificationSource
import org.bitcoinppl.cove_core.CloudBackupVerificationState
import org.bitcoinppl.cove_core.DeepVerificationFailure
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.SettingsRoute

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
    val isNavigationSettled: Boolean = true,
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

internal sealed class CloudBackupVerificationFeedback {
    data class SuccessFloater(val text: UiText) : CloudBackupVerificationFeedback()

    data class FailureAlert(
        val title: UiText,
        val message: UiText,
    ) : CloudBackupVerificationFeedback()
}

internal fun cloudBackupVerificationFeedback(
    presentation: CloudBackupVerificationPresentation,
): CloudBackupVerificationFeedback? =
    when (presentation) {
        is CloudBackupVerificationPresentation.Completed ->
            if (presentation.source == CloudBackupVerificationSource.ROOT_PROMPT) {
                CloudBackupVerificationFeedback.SuccessFloater(UiText.resource(R.string.cloud_backup_verified_floater))
            } else {
                null
            }

        is CloudBackupVerificationPresentation.Failed ->
            if (presentation.source == CloudBackupVerificationSource.ROOT_PROMPT) {
                CloudBackupVerificationFeedback.FailureAlert(
                    title = UiText.resource(R.string.cloud_backup_verification_failed_title),
                    message = presentation.failure.localizedMessage(),
                )
            } else {
                null
            }

        else -> null
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
    if (!context.isNavigationSettled) return false

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
        val desired = CloudBackupManager.getInstance().rootPrompt.toRootPresentation()

        if (desired == null) {
            requiresPresentationDelay = false
            clearVisiblePresentation()
            return
        }

        if (!isPresentable(desired)) {
            transitionJob?.cancel()
            transitionJob = null
            queuedPresentation = desired
            if (blockers.contains(CloudBackupPresentationBlocker.SETTINGS_LOCAL_MODAL)) {
                requiresPresentationDelay = true
            }
            if (currentPresentation == desired && isPromptBlockedOnlyByNavigationSettling(desired)) {
                return
            }
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
                if (CloudBackupManager.getInstance().rootPrompt.toRootPresentation() != queued) {
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

    private fun isPromptBlockedOnlyByNavigationSettling(presentation: CloudBackupRootPresentation): Boolean {
        if (context.isNavigationSettled) return false

        return isCloudBackupPresentationPresentable(
            presentation = presentation,
            context = context.copy(isNavigationSettled = true),
            hasBlockers = blockers.isNotEmpty(),
        )
    }

    companion object {
        private const val PRESENTATION_DELAY_MS = 800L
    }
}

private fun CloudBackupRootPrompt.toRootPresentation(): CloudBackupRootPresentation? =
    when (this) {
        is CloudBackupRootPrompt.None -> null
        is CloudBackupRootPrompt.ExistingBackupFound -> CloudBackupRootPresentation.ExistingBackupFound(v1, v2)
        is CloudBackupRootPrompt.PasskeyChoice -> CloudBackupRootPresentation.PasskeyChoice(v1)
        is CloudBackupRootPrompt.MissingPasskeyReminder -> CloudBackupRootPresentation.MissingPasskeyReminder
        is CloudBackupRootPrompt.Verification -> CloudBackupRootPresentation.VerificationPrompt
    }

private fun existingPasskeyButtonTitle(hint: CloudBackupPasskeyHint?): UiText =
    hint?.let { UiText.resource(R.string.settings_action_use_existing_passkey_named, it.nameSuffix) }
        ?: UiText.resource(R.string.settings_action_use_existing_passkey)

private fun existingBackupMessage(hint: CloudBackupPasskeyHint?): UiText =
    hint?.let {
        UiText.resource(R.string.cloud_backup_existing_backup_message_named, it.nameSuffix)
    } ?: UiText.resource(R.string.cloud_backup_existing_backup_message)

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
    val androidContext = LocalContext.current
    val lifecycleOwner = LocalLifecycleOwner.current
    var isActivityResumed by remember { mutableStateOf(lifecycleOwner.lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED)) }
    var observedVerificationPresentation by remember { mutableStateOf(manager.verificationPresentation) }
    var successFloaterText by remember { mutableStateOf<String?>(null) }

    val context =
        CloudBackupPresentationContext(
            isActivityResumed = isActivityResumed,
            isUnlocked = !auth.isLocked,
            isInDecoyMode = auth.isInDecoyMode(),
            isCoverPresented = isCoverPresented,
            appHasAlert = app.alertState != null,
            appHasSheet = app.sheetState != null,
            isViewingCloudBackup = app.currentRoute == Route.Settings(SettingsRoute.CloudBackup),
            isNavigationSettled = app.isNavigationSettled,
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

    LaunchedEffect(manager.rootPrompt) {
        coordinator.reconcile()
    }

    LaunchedEffect(manager.verificationPresentation) {
        val presentation = manager.verificationPresentation
        if (presentation == observedVerificationPresentation) return@LaunchedEffect

        observedVerificationPresentation = presentation
        when (val feedback = cloudBackupVerificationFeedback(presentation)) {
            is CloudBackupVerificationFeedback.SuccessFloater -> {
                successFloaterText = feedback.text.resolve(androidContext)
            }
            is CloudBackupVerificationFeedback.FailureAlert -> {
                app.alertState =
                    TaggedItem(
                        AppAlertState.General(
                            title = feedback.title.resolve(androidContext),
                            message = feedback.message.resolve(androidContext),
                        ),
                    )
            }
            null -> Unit
        }
    }

    LaunchedEffect(successFloaterText) {
        val text = successFloaterText ?: return@LaunchedEffect
        delay(SUCCESS_FLOATER_DURATION_MS)
        if (successFloaterText == text) {
            successFloaterText = null
        }
    }

    androidx.compose.runtime.CompositionLocalProvider(
        LocalCloudBackupPresentationCoordinator provides coordinator,
    ) {
        Box(modifier = Modifier.fillMaxSize()) {
            content()
            successFloaterText?.let { text ->
                CloudBackupSuccessFloater(
                    text = text,
                    modifier =
                        Modifier
                            .align(Alignment.TopCenter)
                            .statusBarsPadding()
                            .padding(top = 14.dp, start = 16.dp, end = 16.dp),
                )
            }
        }
    }

    when (val presentation = coordinator.currentPresentation) {
        is CloudBackupRootPresentation.ExistingBackupFound -> {
            ChoiceAlertDialog(
                title = stringResource(R.string.cloud_backup_existing_backup_found_title),
                message = existingBackupMessage(presentation.passkeyHint).asString(),
                choices =
                    listOf(
                        DialogChoice(stringResource(R.string.settings_action_create_new_backup)) {
                            coordinator.dismissCurrentPresentation()
                            manager.dispatch(
                                CloudBackupManagerAction.AcceptEnablePrompt(
                                    CloudBackupEnablePromptChoice.CREATE_NEW,
                                ),
                            )
                        },
                        DialogChoice(stringResource(R.string.settings_action_try_existing_passkey)) {
                            coordinator.dismissCurrentPresentation()
                            manager.dispatch(
                                CloudBackupManagerAction.AcceptEnablePrompt(
                                    CloudBackupEnablePromptChoice.USE_EXISTING,
                                ),
                            )
                        },
                    ),
                onDismiss = {
                    if (!coordinator.consumeDismissEvent()) {
                        manager.dispatch(CloudBackupManagerAction.DiscardPendingEnableCloudBackup)
                    }
                },
                onCancel = {
                    coordinator.dismissCurrentPresentation()
                    manager.dispatch(CloudBackupManagerAction.DiscardPendingEnableCloudBackup)
                },
            )
        }

        is CloudBackupRootPresentation.PasskeyChoice -> {
            ChoiceAlertDialog(
                title = stringResource(R.string.cloud_backup_passkey_choice_title),
                message = stringResource(R.string.cloud_backup_passkey_choice_message),
                choices =
                    listOf(
                        DialogChoice(
                            existingPasskeyButtonTitle(
                                (presentation.intent as? CloudBackupPasskeyChoiceIntent.Enable)?.v2,
                            ).asString(),
                        ) {
                            coordinator.dismissCurrentPresentation()
                            when (val intent = presentation.intent) {
                                is CloudBackupPasskeyChoiceIntent.Enable ->
                                    manager.dispatch(
                                        CloudBackupManagerAction.AcceptEnablePrompt(
                                            CloudBackupEnablePromptChoice.USE_EXISTING,
                                        ),
                                    )
                                is CloudBackupPasskeyChoiceIntent.RepairPasskey ->
                                    manager.dispatch(CloudBackupManagerAction.RepairPasskey)
                            }
                        },
                        DialogChoice(stringResource(R.string.settings_action_create_new_passkey)) {
                            coordinator.dismissCurrentPresentation()
                            when (val intent = presentation.intent) {
                                is CloudBackupPasskeyChoiceIntent.Enable ->
                                    manager.dispatch(
                                        CloudBackupManagerAction.AcceptEnablePrompt(
                                            CloudBackupEnablePromptChoice.CREATE_NEW,
                                        ),
                                    )
                                is CloudBackupPasskeyChoiceIntent.RepairPasskey ->
                                    manager.dispatch(CloudBackupManagerAction.RepairPasskeyNoDiscovery)
                            }
                        },
                    ),
                onDismiss = {
                    if (!coordinator.consumeDismissEvent()) {
                        manager.dispatch(CloudBackupManagerAction.DismissPasskeyChoicePrompt)
                    }
                },
                onCancel = {
                    coordinator.dismissCurrentPresentation()
                    manager.dispatch(CloudBackupManagerAction.DismissPasskeyChoicePrompt)
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
                title = { Text(stringResource(R.string.cloud_backup_passkey_missing_reminder_title)) },
                text = {
                    Text(
                        stringResource(R.string.cloud_backup_passkey_missing_reminder_message),
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
                    ) { Text(stringResource(R.string.settings_action_open_cloud_backup)) }
                },
                dismissButton = {
                    TextButton(
                        onClick = {
                            coordinator.dismissCurrentPresentation()
                            manager.dispatch(CloudBackupManagerAction.DismissMissingPasskeyReminder)
                        },
                    ) { Text(stringResource(R.string.settings_action_not_now)) }
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
                    coordinator.dismissCurrentPresentation()
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
private fun CloudBackupSuccessFloater(
    text: String,
    modifier: Modifier = Modifier,
) {
    Surface(
        modifier = modifier,
        shape = MaterialTheme.shapes.medium,
        color = MaterialTheme.colorScheme.surface,
        tonalElevation = 6.dp,
        shadowElevation = 8.dp,
    ) {
        RowWithCheckIcon(text)
    }
}

@Composable
private fun RowWithCheckIcon(text: String) {
    Row(
        modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp),
        horizontalArrangement = Arrangement.spacedBy(10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            imageVector = Icons.Default.CheckCircle,
            contentDescription = null,
            tint = MaterialTheme.coveColors.systemGreen,
        )
        Text(text, style = MaterialTheme.typography.bodyMedium)
    }
}

@Composable
private fun CloudBackupVerificationPrompt(
    manager: CloudBackupManager,
    onDismiss: () -> Unit,
    onVerify: () -> Unit,
) {
    val isVerifying = manager.verificationState is CloudBackupVerificationState.Running
    val failure =
        if (manager.shouldPromptVerification) {
            null
        } else {
            (manager.verificationState as? CloudBackupVerificationState.Failed)?.v1
        }

    val title =
        when {
            isVerifying -> stringResource(R.string.cloud_backup_verification_prompt_running_title)
            failure != null -> stringResource(R.string.cloud_backup_verification_prompt_failed_title)
            else -> stringResource(R.string.cloud_backup_verification_prompt_title)
        }

    val message =
        when {
            failure != null -> failure.localizedMessage().asString()
            isVerifying ->
                stringResource(R.string.cloud_backup_verification_prompt_running_message)
            else ->
                stringResource(R.string.cloud_backup_verification_prompt_message)
        }

    CloudBackupVerificationPromptDialog(
        title = title,
        message = message,
        isVerifying = isVerifying,
        hasFailure = failure != null,
        onDismissRequest = {
            if (!isVerifying) {
                onDismiss()
            }
        },
        onDismiss = onDismiss,
        onVerify = onVerify,
    )
}

@Composable
internal fun CloudBackupVerificationPromptDialog(
    title: String,
    message: String,
    isVerifying: Boolean,
    hasFailure: Boolean,
    onDismissRequest: () -> Unit,
    onDismiss: () -> Unit,
    onVerify: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismissRequest,
        icon = {
            if (isVerifying) {
                CircularProgressIndicator(
                    strokeWidth = 2.dp,
                    modifier = Modifier.size(24.dp),
                )
            } else {
                Icon(
                    imageVector = if (hasFailure) Icons.Default.Warning else Icons.Default.CheckCircle,
                    contentDescription = null,
                    tint =
                        if (hasFailure) {
                            MaterialTheme.colorScheme.error
                        } else {
                            MaterialTheme.coveColors.systemGreen
                        },
                )
            }
        },
        title = { Text(title) },
        text = { Text(message) },
        confirmButton = {
            TextButton(
                onClick = onVerify,
                enabled = !isVerifying,
            ) {
                Text(
                    if (hasFailure) {
                        stringResource(R.string.action_try_again)
                    } else {
                        stringResource(R.string.settings_action_verify)
                    },
                )
            }
        },
        dismissButton = {
            if (!isVerifying) {
                TextButton(onClick = onDismiss) {
                    Text(stringResource(R.string.settings_action_not_now))
                }
            }
        },
        properties =
            DialogProperties(
                dismissOnBackPress = !isVerifying,
                dismissOnClickOutside = false,
            ),
    )
}

@Composable
internal fun CloudBackupVerificationPromptPreviewContent() {
    CloudBackupVerificationPromptPreviewScaffold {
        CloudBackupVerificationPromptDialog(
            title = stringResource(R.string.cloud_backup_verification_prompt_title),
            message = stringResource(R.string.cloud_backup_verification_prompt_message),
            isVerifying = false,
            hasFailure = false,
            onDismissRequest = {},
            onDismiss = {},
            onVerify = {},
        )
    }
}

@Composable
internal fun CloudBackupVerificationPromptRunningPreviewContent() {
    CloudBackupVerificationPromptPreviewScaffold {
        CloudBackupVerificationPromptDialog(
            title = stringResource(R.string.cloud_backup_verification_prompt_running_title),
            message = stringResource(R.string.cloud_backup_verification_prompt_running_message),
            isVerifying = true,
            hasFailure = false,
            onDismissRequest = {},
            onDismiss = {},
            onVerify = {},
        )
    }
}

@Composable
internal fun CloudBackupVerificationPromptFailurePreviewContent() {
    CloudBackupVerificationPromptPreviewScaffold {
        CloudBackupVerificationPromptDialog(
            title = stringResource(R.string.cloud_backup_verification_prompt_failed_title),
            message = stringResource(R.string.deep_verification_retry),
            isVerifying = false,
            hasFailure = true,
            onDismissRequest = {},
            onDismiss = {},
            onVerify = {},
        )
    }
}

@Composable
private fun CloudBackupVerificationPromptPreviewScaffold(
    content: @Composable () -> Unit,
) {
    CoveTheme(darkTheme = false, dynamicColor = false) {
        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .background(MaterialTheme.colorScheme.background),
        ) {
            content()
        }
    }
}

private const val SUCCESS_FLOATER_DURATION_MS = 2_000L
