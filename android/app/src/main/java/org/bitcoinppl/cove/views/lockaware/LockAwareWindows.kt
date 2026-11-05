// Every other file imports these wrappers instead of the Compose originals, which detekt's
// ForbiddenImport rule enforces, so only this file and the lock screen may import them. The
// wrappers mirror the Compose signatures so callers only change an import, hence the long lists
@file:Suppress("ForbiddenImport", "LongParameterList")

package org.bitcoinppl.cove.views.lockaware

import android.view.View
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.ScrollState
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material3.AlertDialogDefaults
import androidx.compose.material3.BottomSheetDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MenuDefaults
import androidx.compose.material3.ModalBottomSheetDefaults
import androidx.compose.material3.ModalBottomSheetProperties
import androidx.compose.material3.SheetState
import androidx.compose.material3.contentColorFor
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.DpOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.window.DialogWindowProvider
import androidx.compose.ui.window.PopupProperties
import org.bitcoinppl.cove.AuthManager
import androidx.compose.material3.AlertDialog as MaterialAlertDialog
import androidx.compose.material3.DropdownMenu as MaterialDropdownMenu
import androidx.compose.material3.ModalBottomSheet as MaterialModalBottomSheet
import androidx.compose.ui.window.Dialog as ComposeDialog

/**
 * Whether the app lock is up, as [AppLockScope] reports it from [AuthManager.isLocked]
 *
 * Compose dialogs, bottom sheets, and menus open in their own windows above the activity, so the
 * lock screen, which draws inside the activity window, cannot cover them. The wrappers in this file
 * read this value and hide or close their window while it is true. Compositions outside
 * [AppLockScope] see the default of false and show their windows. Those are previews, screenshot
 * tests, and the startup screens shown before bootstrap, when no app lock exists yet
 */
val LocalAppLocked = compositionLocalOf { false }

/**
 * Provides [LocalAppLocked] from [auth] to [content], so the wrappers in this file hide or close
 * their windows while the app lock is up, and drops focus and text selection in the activity
 * window when the app locks
 *
 * `MainActivity` wraps everything it shows after bootstrap in this
 */
@Composable
fun AppLockScope(
    auth: AuthManager,
    content: @Composable () -> Unit,
) {
    CompositionLocalProvider(LocalAppLocked provides auth.isLocked) {
        ClearFocusWhenLocked()
        content()
    }
}

/**
 * A Material 3 [androidx.compose.material3.AlertDialog] whose window is hidden while the app is
 * locked
 *
 * Takes the same parameters. See [HideWindowWhileLocked] for what hiding keeps
 */
@Composable
fun AlertDialog(
    onDismissRequest: () -> Unit,
    confirmButton: @Composable () -> Unit,
    modifier: Modifier = Modifier,
    dismissButton: @Composable (() -> Unit)? = null,
    icon: @Composable (() -> Unit)? = null,
    title: @Composable (() -> Unit)? = null,
    text: @Composable (() -> Unit)? = null,
    shape: Shape = AlertDialogDefaults.shape,
    containerColor: Color = AlertDialogDefaults.containerColor,
    iconContentColor: Color = AlertDialogDefaults.iconContentColor,
    titleContentColor: Color = AlertDialogDefaults.titleContentColor,
    textContentColor: Color = AlertDialogDefaults.textContentColor,
    tonalElevation: Dp = AlertDialogDefaults.TonalElevation,
    properties: DialogProperties = DialogProperties(),
) {
    if (!rememberOpenedUnlocked()) return

    MaterialAlertDialog(
        onDismissRequest = onDismissRequest,
        // the confirm button is the one slot every alert composes
        confirmButton = {
            HideWindowWhileLocked()
            confirmButton()
        },
        modifier = modifier,
        dismissButton = dismissButton,
        icon = icon,
        title = title,
        text = text,
        shape = shape,
        containerColor = containerColor,
        iconContentColor = iconContentColor,
        titleContentColor = titleContentColor,
        textContentColor = textContentColor,
        tonalElevation = tonalElevation,
        properties = properties,
    )
}

/**
 * A Material 3 [androidx.compose.material3.ModalBottomSheet] whose window is hidden while the app
 * is locked
 *
 * Takes the same parameters. See [HideWindowWhileLocked] for what hiding keeps
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ModalBottomSheet(
    onDismissRequest: () -> Unit,
    modifier: Modifier = Modifier,
    sheetState: SheetState = rememberModalBottomSheetState(),
    sheetMaxWidth: Dp = BottomSheetDefaults.SheetMaxWidth,
    sheetGesturesEnabled: Boolean = true,
    shape: Shape = BottomSheetDefaults.ExpandedShape,
    containerColor: Color = BottomSheetDefaults.ContainerColor,
    contentColor: Color = contentColorFor(containerColor),
    tonalElevation: Dp = 0.dp,
    scrimColor: Color = BottomSheetDefaults.ScrimColor,
    dragHandle: @Composable (() -> Unit)? = { BottomSheetDefaults.DragHandle() },
    contentWindowInsets: @Composable () -> WindowInsets = { BottomSheetDefaults.windowInsets },
    properties: ModalBottomSheetProperties = ModalBottomSheetDefaults.properties,
    content: @Composable ColumnScope.() -> Unit,
) {
    if (!rememberOpenedUnlocked()) return

    MaterialModalBottomSheet(
        onDismissRequest = onDismissRequest,
        modifier = modifier,
        sheetState = sheetState,
        sheetMaxWidth = sheetMaxWidth,
        sheetGesturesEnabled = sheetGesturesEnabled,
        shape = shape,
        containerColor = containerColor,
        contentColor = contentColor,
        tonalElevation = tonalElevation,
        scrimColor = scrimColor,
        dragHandle = dragHandle,
        contentWindowInsets = contentWindowInsets,
        properties = properties,
    ) {
        HideWindowWhileLocked()
        content()
    }
}

/**
 * A Compose [androidx.compose.ui.window.Dialog] whose window is hidden while the app is locked
 *
 * Takes the same parameters. See [HideWindowWhileLocked] for what hiding keeps
 */
@Composable
fun Dialog(
    onDismissRequest: () -> Unit,
    properties: DialogProperties = DialogProperties(),
    content: @Composable () -> Unit,
) {
    if (!rememberOpenedUnlocked()) return

    ComposeDialog(
        onDismissRequest = onDismissRequest,
        properties = properties,
    ) {
        HideWindowWhileLocked()
        content()
    }
}

/**
 * A Material 3 [androidx.compose.material3.DropdownMenu] that closes at once while the app is
 * locked
 *
 * Takes the same parameters. A menu holds no state worth keeping, so it leaves composition instead
 * of hiding, and opens again once the user unlocks if [expanded] is still true. It skips the exit
 * animation, which would keep the menu usable over the lock until it finished
 */
@Composable
fun DropdownMenu(
    expanded: Boolean,
    onDismissRequest: () -> Unit,
    modifier: Modifier = Modifier,
    offset: DpOffset = DpOffset(0.dp, 0.dp),
    scrollState: ScrollState = rememberScrollState(),
    properties: PopupProperties = PopupProperties(focusable = true),
    shape: Shape = MenuDefaults.shape,
    containerColor: Color = MenuDefaults.containerColor,
    tonalElevation: Dp = MenuDefaults.TonalElevation,
    shadowElevation: Dp = MenuDefaults.ShadowElevation,
    border: BorderStroke? = null,
    content: @Composable ColumnScope.() -> Unit,
) {
    if (LocalAppLocked.current) return

    MaterialDropdownMenu(
        expanded = expanded,
        onDismissRequest = onDismissRequest,
        modifier = modifier,
        offset = offset,
        scrollState = scrollState,
        properties = properties,
        shape = shape,
        containerColor = containerColor,
        tonalElevation = tonalElevation,
        shadowElevation = shadowElevation,
        border = border,
        content = content,
    )
}

/**
 * Whether this dialog or sheet has opened, which it first does once the app is unlocked
 *
 * A window that would first open while locked waits for unlock instead of opening hidden, because
 * Android draws its first frame before [HideWindowWhileLocked] can hide it. Nothing inside it exists
 * yet, so waiting loses no state. Once open, it stays open through later locks and hides instead
 */
@Composable
private fun rememberOpenedUnlocked(): Boolean {
    val isLocked = LocalAppLocked.current
    val opened = remember { OpenedUnlocked() }

    // recorded only once the unlocked composition commits, so one that Compose abandons cannot let
    // a later locked composition open the window. A plain field, since only a later composition
    // reads it, and the lock change that matters recomposes this scope anyway
    if (!isLocked) SideEffect { opened.value = true }
    return !isLocked || opened.value
}

private class OpenedUnlocked {
    var value = false
}

/**
 * Hides the window this is composed in while the app is locked, and shows it again on unlock
 *
 * Call it from the content of a dialog or sheet window. A hidden window is not drawn, takes no
 * touches or focus, and is left out of what accessibility services read, but its content stays
 * composed. So state, pending work, and activity result launchers registered inside survive the
 * lock, as the screen under the lock screen does. Focus and text selection in it are dropped, see
 * [ClearFocusWhenLocked]
 */
@Composable
private fun HideWindowWhileLocked() {
    ClearFocusWhenLocked()

    val isLocked = LocalAppLocked.current
    val view = LocalView.current
    val window = (view.parent as? DialogWindowProvider)?.window
    val windowRoot = view.rootView
    val windowAnimations = remember(window) { window?.attributes?.windowAnimations }

    DisposableEffect(windowRoot, window, isLocked) {
        if (isLocked) {
            // hiding a window plays its exit animation from the last frame drawn, which would leave
            // the content fading out above the lock screen
            window?.setWindowAnimations(0)
            windowRoot.visibility = View.INVISIBLE
        } else {
            windowRoot.visibility = View.VISIBLE
            windowAnimations?.let { window?.setWindowAnimations(it) }
        }

        onDispose {}
    }
}

/**
 * Clears focus in the window this is composed in when the app locks, which drops text selection
 *
 * Selection handles and the copy and paste toolbar open their own windows, which the lock window
 * does not cover, and selected text stays copyable through them. Text fields and selection
 * containers drop their selection, handles, and toolbar when they lose focus. Each window has its
 * own focus, so the activity window and every dialog or sheet clears its own. Focus is not restored
 * on unlock, so the user taps the field again
 */
@Composable
private fun ClearFocusWhenLocked() {
    val isLocked = LocalAppLocked.current
    val focusManager = LocalFocusManager.current

    DisposableEffect(isLocked, focusManager) {
        // forced, so a node that captured focus lets go of it too
        if (isLocked) focusManager.clearFocus(force = true)

        onDispose {}
    }
}
